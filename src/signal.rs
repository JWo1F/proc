use nix::sys::signal::Signal;
use tokio::process::Child;

pub trait ChildSignal {
  fn signal(&self, signal: Signal);
}

impl ChildSignal for Child {
  fn signal(&self, signal: Signal) {
    if let Some(pid) = self.id() {
      let gid = -(pid as i32);
      let res = nix::sys::signal::kill(nix::unistd::Pid::from_raw(gid), signal);

      if res.is_err() {
        eprintln!("Error sending signal to process {}: {}", pid, res.unwrap_err());
      }
    }
  }
}
