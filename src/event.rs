use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use compact_str::CompactString;
use crossterm::event::{
    self, Event as CrosstermEvent, KeyEvent, KeyEventKind, MouseEvent, MouseEventKind,
};
use tracing::Level;

use crate::{
    config::SlimConfig,
    dispatcher::Dispatcher,
    domain::{
        IssueDetailPage, IssueStatus, ListViewKind, SentryEventDetail, SentryIssue,
        SentryMutation, SentryProject,
    },
    id::IssueId,
    result::SlimError,
};

#[derive(Debug, Clone)]
pub struct PollBatchPayload {
    pub projects: Vec<SentryProject>,
    pub issues: Vec<SentryIssue>,
    pub synced_at: DateTime<Utc>,
    pub is_initial: bool,
}

#[derive(Debug, Clone)]
pub enum SlimEvent {
    // Terminal inputs & frame ticks
    InputKey(KeyEvent),
    InputMouse(MouseEvent),
    Tick,

    // Lifecycle & modal controls
    Shutdown,
    ConfigOpen,
    ConfigClose,
    ConfigApply {
        config: SlimConfig,
        persist_token: bool,
    },
    HelpOpen,
    HelpClose,

    // Main List View navigation & filters
    SwitchListView(ListViewKind),
    CycleProjectScope {
        reverse: bool,
    },
    CycleIssueSort,
    FilterInputStart,
    FilterInputChar(CompactString),
    FilterInputBackspace,
    FilterInputApply(CompactString),
    FilterInputClear,

    // Cursor movement in active list view
    CursorNext,
    CursorPrevious,
    CursorPageDown,
    CursorPageUp,
    CursorFirst,
    CursorLast,
    SelectAndOpenIssue {
        row_index: usize,
        issue_id: IssueId,
    },
    EscapeOrBack,

    // Multi-Page Issue Detail View navigation
    OpenSelectedIssue,
    OpenIssueDetail(IssueId),
    CloseIssueDetail,
    SwitchDetailPage(IssueDetailPage),
    NextDetailPage,
    PrevDetailPage,
    DetailScrollDown(u16),
    DetailScrollUp(u16),
    DetailScrollTop,
    DetailScrollBottom,
    ToggleInAppFramesOnly,
    ToggleExpandFrameVars,

    // Quick Actions & Clipboard
    MutateSelectedStatus(IssueStatus),
    SubmitMutation(SentryMutation),
    IssueStatusUpdated {
        issue_id: IssueId,
        short_id: CompactString,
        status: IssueStatus,
    },
    MutationFailed(CompactString),
    CopySelectedShortId,
    CopySelectedMarkdownSummary,
    OpenSelectedUrl,
    CopyToClipboard {
        label: CompactString,
        value: String,
    },
    OpenUrl(CompactString),

    // Synchronization lifecycle & callbacks
    ManualRefresh,
    PollBatchLoaded(Box<PollBatchPayload>),
    IssueEventLoaded(Box<SentryEventDetail>),
    SyncError(CompactString),
    AppError(SlimError),

    // Notifications & logging
    FlushNotificationBuffer,
    NotificationDismiss,
    NotificationRecallLast,
    ActivityNotice(CompactString),
    LogEntry(CompactString),
    LogLevelChanged(Level),
}

#[derive(Debug)]
pub struct EventHandler {
    sender: mpsc::Sender<SlimEvent>,
    receiver: mpsc::Receiver<SlimEvent>,
    fast_tick: Arc<AtomicBool>,
    _handler: thread::JoinHandle<()>,
}

impl EventHandler {
    pub fn new(tick_rate: Duration) -> Self {
        let (sender, receiver) = mpsc::channel();
        let fast_tick = Arc::new(AtomicBool::new(false));

        let handler = {
            let sender = sender.clone();
            let fast_tick = Arc::clone(&fast_tick);
            thread::spawn(move || {
                let mut last_tick = Instant::now();
                loop {
                    let current_rate = if fast_tick.load(Ordering::Relaxed) {
                        tick_rate
                    } else {
                        Duration::from_secs(15)
                    };
                    let timeout = current_rate
                        .checked_sub(last_tick.elapsed())
                        .unwrap_or(Duration::ZERO)
                        .min(Duration::from_millis(50));

                    if event::poll(timeout).unwrap_or(false) {
                        Self::apply_event(&sender);
                    }

                    if last_tick.elapsed() >= current_rate {
                        sender.dispatch(SlimEvent::Tick);
                        last_tick = Instant::now();
                    }
                }
            })
        };

        Self {
            sender,
            receiver,
            fast_tick,
            _handler: handler,
        }
    }

    pub fn set_fast_tick(&self, enabled: bool) {
        self.fast_tick.store(enabled, Ordering::Relaxed);
    }

    pub fn sender(&self) -> mpsc::Sender<SlimEvent> {
        self.sender.clone()
    }

    pub fn next(&self) -> Result<SlimEvent, mpsc::RecvError> {
        self.receiver.recv()
    }

    pub fn try_next(&self) -> Option<SlimEvent> {
        self.receiver.try_recv().ok()
    }

    fn apply_event(sender: &mpsc::Sender<SlimEvent>) {
        if let Ok(ev) = event::read() {
            match ev {
                CrosstermEvent::Key(key) if key.kind == KeyEventKind::Press => {
                    sender.dispatch(SlimEvent::InputKey(key));
                }
                CrosstermEvent::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::Down(_)
                    | MouseEventKind::ScrollDown
                    | MouseEventKind::ScrollUp => {
                        sender.dispatch(SlimEvent::InputMouse(mouse));
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
}
