//! A host with no way to signal a process by pid.

pub fn install_exit_hooks() {}

pub fn adopt(_pid: u32) {}

pub fn set_quit_channel(_monitor: &std::net::TcpStream) {}

pub fn terminate(_pid: i32) {}

pub fn kill(_pid: i32) {}

pub fn keep_across_exec(_fd: i32) {}

pub fn is_alive(_pid: i32) -> bool {
    false
}
