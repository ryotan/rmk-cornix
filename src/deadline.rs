//! Runs a [`DeadlineProcessor`] under `#[register_processor(event)]`.
//!
//! RMK's `#[register_processor]` only offers `event` (which runs
//! [`Processor::process_loop`]) and `poll` (a fixed-interval ticker). A
//! deadline-driven processor needs its own loop, so this wrapper overrides
//! `process_loop` with [`DeadlineProcessor::deadline_loop`]. The `subscriber`
//! and `process` delegations only satisfy the `Processor` trait bound;
//! `deadline_loop` calls `P::subscriber` and `P::process` itself.

use rmk::core_traits::Runnable;
use rmk::event::EventSubscriber;
use rmk::processor::{DeadlineProcessor, Processor};

pub struct DeadlineDriven<P>(pub P);

impl<P: DeadlineProcessor> Runnable for DeadlineDriven<P> {
    async fn run(&mut self) -> ! {
        self.0.deadline_loop().await
    }
}

impl<P: DeadlineProcessor> Processor for DeadlineDriven<P> {
    type Event = P::Event;

    fn subscriber() -> impl EventSubscriber<Event = Self::Event> {
        P::subscriber()
    }

    async fn process(&mut self, event: Self::Event) {
        self.0.process(event).await
    }

    async fn process_loop(&mut self) -> ! {
        self.0.deadline_loop().await
    }
}
