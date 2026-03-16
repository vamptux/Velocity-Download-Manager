#[derive(Default)]
pub struct WakeLockController {
    active_downloads: usize,
    lock_held: bool,
}

impl WakeLockController {
    pub fn retain(&mut self) {
        self.active_downloads = self.active_downloads.saturating_add(1);
        if self.active_downloads == 1 {
            self.lock_held = acquire_wake_lock();
        }
    }

    pub fn release(&mut self) {
        if self.active_downloads == 0 {
            return;
        }
        self.active_downloads = self.active_downloads.saturating_sub(1);
        if self.active_downloads == 0 && self.lock_held {
            release_wake_lock();
            self.lock_held = false;
        }
    }
}

#[cfg(target_os = "windows")]
fn acquire_wake_lock() -> bool {
    use winapi::um::winbase::SetThreadExecutionState;
    const ES_AWAYMODE_REQUIRED: u32 = 0x0000_0040;
    const ES_CONTINUOUS: u32 = 0x8000_0000;
    const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;

    unsafe {
        SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_AWAYMODE_REQUIRED) != 0
    }
}

#[cfg(not(target_os = "windows"))]
fn acquire_wake_lock() -> bool {
    true
}

#[cfg(target_os = "windows")]
fn release_wake_lock() {
    use winapi::um::winbase::SetThreadExecutionState;
    const ES_CONTINUOUS: u32 = 0x8000_0000;

    unsafe {
        let _ = SetThreadExecutionState(ES_CONTINUOUS);
    }
}

#[cfg(not(target_os = "windows"))]
fn release_wake_lock() {}
