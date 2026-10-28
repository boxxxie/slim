use std::sync::mpsc;

use crate::event::SlimEvent;

pub trait Dispatcher {
    fn dispatch(&self, event: SlimEvent);
}

impl Dispatcher for mpsc::Sender<SlimEvent> {
    fn dispatch(&self, event: SlimEvent) {
        let _ = self.send(event);
    }
}
