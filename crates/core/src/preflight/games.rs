use std::collections::HashSet;
use std::time::{Duration, Instant};

use sysinfo::{Signal, System};

use super::scan::{
    executable_name, is_cef_child_arg, is_windows_exe, refresh, scan, ProcKind,
};
#[cfg(not(target_os = "macos"))]
use super::scan::{belongs_to, owned_prefixes};
use super::shutdown::{signal_all, Disposition};
use crate::paths::Layout;

/// How often the prefix is re-scanned while waiting for a handed-off game to
/// start. Slower than [`super::POLL`]: a cold client takes most of a minute.
const HANDOFF_POLL: Duration = Duration::from_millis(500);

/// Wine's own processes, in every prefix and nobody's game. The
/// before-snapshot catches most, but Wine starts services on demand too.
///
/// The last four outlive the game: `steam.exe` is Proton's shim, `xalia.exe`
/// GE-Proton's accessibility helper, `lsass.exe` comes with a login and
/// `rundll32.exe` with the shell. Counted as games, they refused every later Play.
const WINE_SERVICES: [&str; 16] = [
    "services",
    "winedevice",
    "plugplay",
    "explorer",
    "rpcss",
    "svchost",
    "conhost",
    "start",
    "wineboot",
    "winemenubuilder",
    "tabtip",
    "msiexec",
    "steam",
    "xalia",
    "lsass",
    "rundll32",
];

/// A process in the prefix that is neither Battle.net's nor Wine's: after a
/// hand-off, the game, whatever title it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameProcess {
    pub pid: u32,
    pub name: String,
}

/// Whether a Battle.net client is already up in this prefix, and can take a
/// launch command directly.
pub fn client_running(layout: &Layout) -> bool {
    scan(layout).iter().any(|p| p.kind == ProcKind::BattleNet)
}

/// Processes in the prefix that are not Battle.net's and not Wine's own.
pub fn games_running(layout: &Layout) -> Vec<GameProcess> {
    let mut system = System::new();
    refresh(&mut system);
    collect_games(&system, layout)
}

/// Block until `pid` exits, on a pidfd: polling [`games_running`] instead
/// cost a whole process scan (30-40 ms of CPU) every 150 ms of a session.
/// An error means no pidfd could be had, and the caller has to poll.
#[cfg(target_os = "linux")]
pub fn wait_for_exit(pid: u32) -> std::io::Result<()> {
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    // SAFETY: pidfd_open(2) takes a pid and flags, returning a new fd or -1.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, libc::c_long::from(pid), 0) };
    if fd < 0 {
        let e = io::Error::last_os_error();
        return match e.raw_os_error() {
            Some(libc::ESRCH) => Ok(()),
            _ => Err(e),
        };
    }
    #[allow(clippy::cast_possible_truncation, reason = "a file descriptor fits a c_int")]
    // SAFETY: a fresh descriptor nothing else owns.
    let pidfd = unsafe { OwnedFd::from_raw_fd(fd as libc::c_int) };
    let mut poll = libc::pollfd { fd: pidfd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    loop {
        // SAFETY: one valid pollfd, no timeout. Readable once the process is gone.
        if unsafe { libc::poll(&mut poll, 1, -1) } >= 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// Block until `pid` exits, on a kqueue `EVFILT_PROC` note. Same contract as
/// the Linux pidfd path: `ESRCH` means it is already gone.
#[cfg(target_os = "macos")]
pub fn wait_for_exit(pid: u32) -> std::io::Result<()> {
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    let kq = unsafe { libc::kqueue() };
    if kq < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a fresh kqueue descriptor nothing else owns.
    let kq = unsafe { OwnedFd::from_raw_fd(kq) };
    let change = libc::kevent {
        ident: pid as libc::uintptr_t,
        filter: libc::EVFILT_PROC,
        flags: libc::EV_ADD | libc::EV_ONESHOT,
        fflags: libc::NOTE_EXIT,
        data: 0,
        udata: std::ptr::null_mut(),
    };
    // Register the filter. A process that has already exited is ESRCH.
    let added = unsafe {
        libc::kevent(
            kq.as_raw_fd(),
            &change,
            1,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
        )
    };
    if added < 0 {
        let e = io::Error::last_os_error();
        return match e.raw_os_error() {
            Some(libc::ESRCH) => Ok(()),
            _ => Err(e),
        };
    }
    let mut event = unsafe { std::mem::zeroed::<libc::kevent>() };
    loop {
        let n = unsafe {
            libc::kevent(
                kq.as_raw_fd(),
                std::ptr::null(),
                0,
                &mut event,
                1,
                std::ptr::null(),
            )
        };
        if n >= 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn wait_for_exit(_pid: u32) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "wait_for_exit is only implemented on Linux and macOS",
    ))
}

fn collect_games(system: &System, layout: &Layout) -> Vec<GameProcess> {
    #[cfg(target_os = "macos")]
    {
        collect_native_games(system, layout)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let prefixes = owned_prefixes(layout);

        system
            .processes()
            .iter()
            .filter_map(|(pid, proc)| {
                if proc.thread_kind().is_some() {
                    return None;
                }
                let fallback = proc.name().to_string_lossy();
                let name = names_a_game(proc.cmd(), &fallback)?;
                if !belongs_to(proc.environ(), &prefixes) {
                    return None;
                }
                Some(GameProcess {
                    pid: pid.as_u32(),
                    name,
                })
            })
            .collect()
    }
}

/// Native macOS: a game is a process whose executable lives under a WoW
/// install we own. There is no Wine prefix to attribute by environment.
#[cfg(target_os = "macos")]
fn collect_native_games(system: &System, layout: &Layout) -> Vec<GameProcess> {
    let roots: Vec<std::path::PathBuf> = layout
        .libraries()
        .all()
        .iter()
        .map(|library| layout.wow_install_in(library))
        .collect();

    system
        .processes()
        .iter()
        .filter_map(|(pid, proc)| {
            if proc.thread_kind().is_some() {
                return None;
            }
            let exe = proc.exe()?;
            if !roots.iter().any(|root| exe.starts_with(root)) {
                return None;
            }
            let name = exe.file_name()?.to_string_lossy().into_owned();
            let lower = name.to_ascii_lowercase();
            if lower.contains("blizzard error") || lower.contains("crashpad") {
                return None;
            }
            Some(GameProcess { pid: pid.as_u32(), name })
        })
        .collect()
}

/// The executable's name, when a process could be a game at all: not
/// Battle.net's, not Wine's, not a browser child. Ownership is checked elsewhere.
pub(super) fn names_a_game(argv: &[std::ffi::OsString], fallback: &str) -> Option<String> {
    if ProcKind::classify(argv, fallback).is_some() {
        return None;
    }
    // Proton's `python3` and wrappers like `gamemoderun` carry the prefix too,
    // but a game under Wine has a Windows executable as `argv[0]`.
    if argv.first().is_some_and(|a| !a.is_empty() && !is_windows_exe(a)) {
        return None;
    }
    let name = executable_name(argv, fallback);
    let stem = name.to_ascii_lowercase();
    if WINE_SERVICES.contains(&stem.trim_end_matches(".exe")) {
        return None;
    }
    // A CEF or Electron child re-execs its parent, and the parent is the
    // process that represents the game.
    if argv.iter().skip(1).any(is_cef_child_arg) {
        return None;
    }
    Some(name)
}

/// A game process and how it was made to stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stopped {
    pub game: GameProcess,
    pub disposition: Disposition,
}

/// Stop one running game, and only it. Not [`super::shutdown`], which takes
/// Battle.net and every other game down with it: `SIGTERM`, then `SIGKILL`
/// after `grace`. `None` when `pid` is no longer a game of this prefix (a pid
/// can be reused). Lifecycle only: nothing reads or writes the game.
pub fn stop_game(layout: &Layout, pid: u32, grace: Duration) -> Option<Stopped> {
    let mut system = System::new();
    refresh(&mut system);
    let game = collect_games(&system, layout).into_iter().find(|g| g.pid == pid)?;

    let disposition = match signal_all(&mut system, layout, &[pid], Signal::Term, grace).is_empty() {
        true => Disposition::Terminated,
        false => match signal_all(&mut system, layout, &[pid], Signal::Kill, grace).is_empty() {
            true => Disposition::Killed,
            false => Disposition::Survived,
        },
    };
    Some(Stopped { game, disposition })
}

/// Wait for a handed-off game to start.
///
/// `before` is [`games_running`] from before the hand-off, so a game already
/// playing does not count. `None` means nothing started in time, usually a
/// client at its login screen. Needed because the client exits once the game
/// is up (`GameLaunchWindowBehavior: 2`).
pub fn wait_for_game_start(
    layout: &Layout,
    before: &[GameProcess],
    appear_within: Duration,
) -> Option<GameProcess> {
    let known: HashSet<u32> = before.iter().map(|p| p.pid).collect();
    let deadline = Instant::now() + appear_within;

    loop {
        if let Some(found) = games_running(layout).into_iter().find(|p| !known.contains(&p.pid)) {
            return Some(found);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(HANDOFF_POLL);
    }
}
