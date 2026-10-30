//! Console logging (logger.js); the file logger comes from
//! droneengage_server_common.
//!
//! logger.js decides whether to also write to the file logger when it is
//! first required, which in the Node.js server is before server.js creates
//! that logger: module log lines only ever reach the console. The file gets
//! just server.js's own entries ("System Started.", "SIGINT."), and `Log`
//! keeps it that way.

pub use droneengage_server_common::logger::{node_stringify, FileLogger};

use crate::colors as c;

/// logger.js: `[INFO]`/`[WARN]`/`[ERROR]` console lines, `[DEBUG]` only
/// with debug_logging.
#[derive(Clone, Default)]
pub struct Log {
    m_debug: bool,
    m_silent: bool,
}

impl Log {
    pub fn new(p_debug: bool) -> Log {
        Log { m_debug: p_debug, m_silent: false }
    }

    /// A logger that prints nothing (tests).
    pub fn silent() -> Log {
        Log { m_debug: false, m_silent: true }
    }

    pub fn debug_enabled(&self) -> bool {
        self.m_debug
    }

    pub fn info(&self, p_message: &str) {
        if !self.m_silent {
            println!("{}[INFO] {}{}", c::B_SUCCESS, c::RESET, p_message);
        }
    }

    pub fn error(&self, p_message: &str) {
        if !self.m_silent {
            println!("{}[ERROR] {}{}", c::ERROR, c::RESET, p_message);
        }
    }

    pub fn warn(&self, p_message: &str) {
        if !self.m_silent {
            println!("{}[WARN] {}{}", c::FG_YELLOW, c::RESET, p_message);
        }
    }

    pub fn debug(&self, p_message: &str) {
        if !self.m_silent && self.m_debug {
            println!("{}[DEBUG] {}{}", c::FG_CYAN, c::RESET, p_message);
        }
    }
}
