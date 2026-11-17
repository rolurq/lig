use std::ffi::OsString;
use std::mem::size_of;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

fn path(chars: impl IntoIterator<Item = libc::c_char>) -> Option<PathBuf> {
    let bytes: Vec<u8> = chars.into_iter().take_while(|&c| c != 0).map(|c| c as u8).collect();
    (!bytes.is_empty()).then(|| PathBuf::from(OsString::from_vec(bytes)))
}

fn bsd(pid: u32) -> Option<libc::proc_bsdinfo> {
    let pid = i32::try_from(pid).ok()?;
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let n = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size_of::<libc::proc_bsdinfo>() as i32,
        )
    };
    (n == size_of::<libc::proc_bsdinfo>() as i32).then_some(info)
}

pub fn cwd(pid: u32) -> Option<PathBuf> {
    let pid = i32::try_from(pid).ok()?;
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let n = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            (&mut info as *mut libc::proc_vnodepathinfo).cast(),
            size_of::<libc::proc_vnodepathinfo>() as i32,
        )
    };
    (n == size_of::<libc::proc_vnodepathinfo>() as i32)
        .then(|| path(info.pvi_cdir.vip_path.into_iter().flatten()))
        .flatten()
}

pub fn executable(pid: u32) -> Option<PathBuf> {
    let pid = i32::try_from(pid).ok()?;
    let mut buf = [0; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (n > 0).then(|| path(buf)).flatten()
}

pub fn children(pid: u32) -> Vec<u32> {
    let Ok(pid) = i32::try_from(pid) else { return vec![] };
    let size = unsafe { libc::proc_listchildpids(pid, std::ptr::null_mut(), 0) };
    if size <= 0 {
        return vec![];
    }
    let mut pids = vec![0i32; (size as usize + 32).min(65536)];
    let n = unsafe { libc::proc_listchildpids(pid, pids.as_mut_ptr().cast(), (pids.len() * size_of::<i32>()) as i32) };
    pids.into_iter().take(n.max(0) as usize).filter_map(|p| u32::try_from(p).ok().filter(|&p| p > 0)).collect()
}

// ABI from Apple's sys/proc_info.h; libc provides vnode_info_path but not these wrappers.
#[repr(C)]
struct FileInfo {
    openflags: u32,
    status: u32,
    offset: libc::off_t,
    kind: i32,
    guardflags: u32,
}
#[repr(C)]
struct VnodeFdInfo {
    file: FileInfo,
    vnode: libc::vnode_info_path,
}

fn has_tty(pid: u32, tty: &Path) -> bool {
    let Some(info) = bsd(pid) else { return false };
    let mut fds: Vec<libc::proc_fdinfo> = (0..(info.pbi_nfiles as usize + 32).min(65536))
        .map(|_| libc::proc_fdinfo { proc_fd: -1, proc_fdtype: 0 })
        .collect();
    let n = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDLISTFDS,
            0,
            fds.as_mut_ptr().cast(),
            (fds.len() * size_of::<libc::proc_fdinfo>()) as i32,
        )
    };
    fds.iter().take(n.max(0) as usize / size_of::<libc::proc_fdinfo>()).any(|fd| {
        // PROX_FDTYPE_VNODE = 1; ignore stderr-only prompt helpers as on Linux.
        if fd.proc_fd == 2 || fd.proc_fdtype != 1 {
            return false;
        }
        let mut vnode: VnodeFdInfo = unsafe { std::mem::zeroed() };
        let n = unsafe {
            libc::proc_pidfdinfo(
                pid as i32,
                fd.proc_fd,
                2,
                (&mut vnode as *mut VnodeFdInfo).cast(),
                size_of::<VnodeFdInfo>() as i32,
            )
        };
        n == size_of::<VnodeFdInfo>() as i32
            && path(vnode.vnode.vip_path.into_iter().flatten()).is_some_and(|p| p == tty || p == Path::new("/dev/tty"))
    })
}

pub fn foreign_program(shell: u32, tty: &Path) -> bool {
    let Some(shell_info) = bsd(shell) else { return false };
    let mut todo = children(shell);
    let mut seen = std::collections::HashSet::new();
    while let Some(pid) = todo.pop() {
        if !seen.insert(pid) || seen.len() > 4096 {
            continue;
        }
        todo.extend(children(pid));
        let Some(info) = bsd(pid) else { continue };
        let comm = path(info.pbi_comm);
        if info.pbi_pgid != shell_info.pbi_pgid
            || comm.as_deref().is_some_and(|p| matches!(p.to_str(), Some("zsh" | "bash" | "sh" | "dash")))
        {
            continue;
        }
        if has_tty(pid, tty) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod native_tests {
    use super::*;
    #[test]
    fn detects_tty_descriptors() {
        let pair = portable_pty::native_pty_system().openpty(portable_pty::PtySize::default()).unwrap();
        let tty = pair.master.tty_name().unwrap();
        let mut cmd = portable_pty::CommandBuilder::new("/bin/sleep");
        cmd.arg("10");
        let mut child = pair.slave.spawn_command(cmd).unwrap();
        let present = has_tty(child.process_id().unwrap(), &tty);
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(present);
    }
    #[test]
    fn stderr_only_and_ttyless_processes_do_not_hold_the_tty() {
        let pair = portable_pty::native_pty_system().openpty(portable_pty::PtySize::default()).unwrap();
        let tty = pair.master.tty_name().unwrap();
        let tty_file = || std::fs::OpenOptions::new().read(true).write(true).open(&tty).unwrap();
        let spawn = |stderr: std::process::Stdio| {
            std::process::Command::new("/bin/sleep")
                .arg("10")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(stderr)
                .spawn()
                .unwrap()
        };
        let mut stderr_only = spawn(tty_file().into());
        let mut ttyless = spawn(std::process::Stdio::null());
        let found = (has_tty(stderr_only.id(), &tty), has_tty(ttyless.id(), &tty));
        for child in [&mut stderr_only, &mut ttyless] {
            child.kill().unwrap();
            child.wait().unwrap();
        }
        assert_eq!(found, (false, false));
    }

    fn fake_shell(script: &str, tty: &Path) -> std::process::Child {
        use std::os::unix::process::CommandExt;
        let child = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .env("TTY_PATH", tty)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while children(child.id()).is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        child
    }

    fn foreign_in(script: &str, expected: bool) -> bool {
        let pair = portable_pty::native_pty_system().openpty(portable_pty::PtySize::default()).unwrap();
        let tty = pair.master.tty_name().unwrap();
        let mut shell = fake_shell(script, &tty);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut found = foreign_program(shell.id(), &tty);
        while found != expected && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
            found = foreign_program(shell.id(), &tty);
        }
        unsafe { libc::kill(-(shell.id() as i32), libc::SIGKILL) };
        shell.wait().unwrap();
        found
    }

    #[test]
    fn program_reading_the_tty_in_the_shells_group_is_foreign() {
        assert!(foreign_in(r#"/bin/sleep 10 < "$TTY_PATH" & wait"#, true));
        assert!(foreign_in(r#"/bin/sh -c '/bin/sleep 10 < "$TTY_PATH"; :' & wait"#, true));
    }

    #[test]
    fn shells_and_stderr_only_helpers_are_not_foreign() {
        assert!(!foreign_in(r#"/bin/zsh -fc 'read x' < "$TTY_PATH" & wait"#, false));
        assert!(!foreign_in(r#"/bin/sleep 10 2> "$TTY_PATH" & wait"#, false));
    }

    #[test]
    fn inspect_self_and_child() {
        let pid = std::process::id();
        assert_eq!(cwd(pid).unwrap().canonicalize().unwrap(), std::env::current_dir().unwrap().canonicalize().unwrap());
        assert_eq!(executable(pid).unwrap(), std::env::current_exe().unwrap());
        let mut child = std::process::Command::new("/bin/sleep").arg("10").spawn().unwrap();
        let found = children(pid).contains(&child.id());
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(found);
        assert!(cwd(u32::MAX).is_none());
    }
}
