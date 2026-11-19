use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilePhase {
    CheckInstalled,
    CheckUpdate,
    Stage,
}

impl FilePhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CheckInstalled => "check_installed",
            Self::CheckUpdate => "check_update",
            Self::Stage => "stage",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FileProgress {
    pub phase: FilePhase,
    pub done: usize,
    pub total: usize,
}

// Publish the phase and its counters together. The UI can read between any two files.
#[derive(Default)]
pub(super) struct RestartProgress(Mutex<Option<FileProgress>>);

impl RestartProgress {
    pub fn begin(&self, phase: FilePhase, total: usize) {
        *self.0.lock().unwrap() = Some(FileProgress { phase, done: 0, total });
    }

    pub fn file_done(&self) {
        if let Some(progress) = self.0.lock().unwrap().as_mut() {
            progress.done += 1;
        }
    }

    pub fn snapshot(&self) -> Option<FileProgress> {
        *self.0.lock().unwrap()
    }
}
