pub mod depot_downloader;
pub mod depot_watch;
pub mod steam_console;

/// Windows process creation flags used when spawning helper processes.
#[cfg(windows)]
pub mod windows {
    /// Open the new console window for DepotDownloader batches.
    pub const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    /// Hide the console window for `cmd /c start steam://...` launches.
    pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
}
