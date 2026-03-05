use nix::sys::signal::Signal;
use tokio::process::Child;

pub trait ChildSignal {
  fn signal(&self, signal: Signal);
}

impl ChildSignal for Child {
  fn signal(&self, signal: Signal) {
    if let Some(pid) = self.id() {
      let gid = -(pid as i32);

      if let Err(err) = nix::sys::signal::kill(nix::unistd::Pid::from_raw(gid), signal) {
        eprintln!("Error sending signal to process {}: {}", pid, err);
      }
    }
  }
}
