use std::{io, panic};

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{Terminal, backend::Backend};

use crate::{
    event::EventHandler,
    result::{Result, SlimError},
};

#[derive(Debug)]
pub struct Tui<B: Backend> {
    terminal: Terminal<B>,
    pub events: EventHandler,
}

impl<B: Backend> Tui<B> {
    pub fn new(terminal: Terminal<B>, events: EventHandler) -> Self {
        Self { terminal, events }
    }

    pub fn init(&mut self) -> Result<()> {
        terminal::enable_raw_mode().map_err(|e| SlimError::GeneralError(e.to_string().into()))?;
        crossterm::execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)
            .map_err(|e| SlimError::GeneralError(e.to_string().into()))?;

        let panic_hook = panic::take_hook();
        panic::set_hook(Box::new(move |panic_info| {
            Self::reset().expect("failed to reset the terminal");
            panic_hook(panic_info);
        }));

        self.terminal
            .hide_cursor()
            .map_err(|e| SlimError::GeneralError(e.to_string().into()))?;
        self.terminal
            .clear()
            .map_err(|e| SlimError::GeneralError(e.to_string().into()))?;
        Ok(())
    }

    pub fn draw_frame(&mut self, mut render_fn: impl FnMut(&mut ratatui::Frame)) -> Result<()> {
        self.terminal
            .draw(|frame| render_fn(frame))
            .map_err(|e| SlimError::GeneralError(e.to_string().into()))?;
        Ok(())
    }

    fn reset() -> Result<()> {
        terminal::disable_raw_mode().map_err(|e| SlimError::GeneralError(e.to_string().into()))?;
        crossterm::execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)
            .map_err(|e| SlimError::GeneralError(e.to_string().into()))?;
        Ok(())
    }

    pub fn exit(&mut self) -> Result<()> {
        Self::reset()?;
        self.terminal
            .show_cursor()
            .map_err(|e| SlimError::GeneralError(e.to_string().into()))?;
        Ok(())
    }
}
