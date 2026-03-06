use nix::sys::signal::Signal;
use tokio::process::Child;

/// Extension trait for sending Unix signals to a child's entire process group.
pub trait ChildSignal {
  fn signal(&self, signal: Signal);
}

impl ChildSignal for Child {
  /// Send `signal` to the process group (negative PID) so that the child
  /// and all its descendants receive it.
  fn signal(&self, signal: Signal) {
    if let Some(pid) = self.id() {
      let gid = -(pid as i32);

      if let Err(err) = nix::sys::signal::kill(nix::unistd::Pid::from_raw(gid), signal) {
        eprintln!("Error sending signal to process {}: {}", pid, err);
      }
    }
  }
}
