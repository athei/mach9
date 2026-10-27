//! Native dialogs.
//!
//! These run in the launcher process itself, and they have to: a plain child
//! process cannot take focus while the app that spawned it was launched from
//! Finder, so its panel draws but every click is spent on an activation that
//! never lands. Only the app process has the standing to activate.
//!
//! The order below is the other half of that, and is easy to get wrong: check
//! in with `finishLaunching`, put the window on screen, activate, and only then
//! run the modal. Activating with nothing on screen does nothing, and without
//! checking in the process counts as still starting up, which paints the wait
//! cursor over everything outside the dialog.
//!
//! The cost is that the process stays registered with the window server for
//! good, which is why a launch that shows a dialog does not go on to start the
//! game: it finishes the setup and starts a fresh instance of the app instead.
//! `used_gui` is how `main` knows to do that.

use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, Message as _, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol},
    sel,
};
use objc2_app_kit::{
    NSAlert, NSAlertStyle, NSApplication, NSApplicationActivationPolicy, NSBackingStoreType,
    NSButton, NSControlStateValueOn, NSModalResponseOK, NSOpenPanel, NSScrollView, NSTextAlignment,
    NSTextField, NSTextView, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString, NSURL, ns_string};

/// Set once anything here has talked to `AppKit`.
static USED_GUI: AtomicBool = AtomicBool::new(false);

/// Whether a dialog was shown, and with it the window server connection that
/// outlives it.
pub fn used_gui() -> bool {
    USED_GUI.load(Ordering::Relaxed)
}

pub fn error(title: &str, message: &str) {
    message_dialog(NSAlertStyle::Critical, title, message);
}

/// A one-button alert. `NSAlert` supplies the OK button and the app's own icon.
fn message_dialog(style: NSAlertStyle, title: &str, message: &str) {
    let Some(app) = become_app() else {
        return;
    };
    let alert = NSAlert::new(MainThreadMarker::from(&*app));
    alert.setAlertStyle(style);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(message));

    alert.window().makeKeyAndOrderFront(None);
    activate(&app);
    alert.runModal();
    // Closing the window is otherwise deferred until something services the run
    // loop, and the launcher goes straight into the next modal or into exec, so
    // the dismissed alert would sit on screen for a second or two.
    alert.window().orderOut(None);
}

/// Say what is needed, then ask for the Windows executable to launch. `None`
/// means the user cancelled.
///
/// The alert comes first and the panel is built after it: a first
/// `NSOpenPanel` in a process takes a couple of seconds of synchronous
/// main-thread work, and building it earlier only leaves the alert frozen on
/// screen while that happens. The pause belongs between the two dialogs, where
/// it reads as the picker opening.
pub fn explain_and_pick(headline: &str, body: &str, start_dir: &Path) -> Option<PathBuf> {
    message_dialog(NSAlertStyle::Informational, headline, body);
    let app = become_app()?;
    open_panel(&app, start_dir)
}

/// The file picker for a Windows executable. `None` means the user cancelled.
fn open_panel(app: &NSApplication, start_dir: &Path) -> Option<PathBuf> {
    let panel = NSOpenPanel::openPanel(MainThreadMarker::from(app));
    let exe_type = NSArray::from_retained_slice(&[NSString::from_str("exe")]);
    let start_url = NSURL::fileURLWithPath(&NSString::from_str(&start_dir.to_string_lossy()));
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    panel.setResolvesAliases(true);
    panel.setDirectoryURL(Some(&start_url));
    // The title is not shown on current macOS; the message above the file list
    // is. Keep it to one short line, the panel sizes its width to it.
    panel.setTitle(Some(&NSString::from_str("Select the game executable")));
    panel.setMessage(Some(&NSString::from_str(
        "Select the game executable, for example Game.exe.",
    )));
    panel.setPrompt(Some(&NSString::from_str("Select")));
    #[allow(deprecated)]
    panel.setAllowedFileTypes(Some(&exe_type));
    // The filter is a hint, not a rule: a client whose executable is named
    // something else is still pickable.
    panel.setAllowsOtherFileTypes(true);

    panel.makeKeyAndOrderFront(None);
    activate(app);
    let response = panel.runModal();
    panel.orderOut(None);
    if response != NSModalResponseOK {
        return None;
    }
    panel
        .URL()
        .and_then(|url| url.path())
        .map(|path| PathBuf::from(path.to_string()))
}

/// What the New Executable window was answered with, as typed. `save` is the
/// "Save as new game" checkbox.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GameForm {
    pub name: String,
    pub executable: String,
    pub arguments: String,
    pub env_text: String,
    pub save: bool,
}

/// The New Executable window: the explanation, the fields, the checkbox, and
/// Cancel/Run. `validate` is asked on Run; its message is shown and the window
/// stays until the answer passes or the user cancels, when this is `None`.
pub fn game_form(
    app_name: &str,
    explanation: &str,
    start_dir: &Path,
    validate: &dyn Fn(&GameForm) -> Result<(), String>,
) -> Option<GameForm> {
    let app = become_app()?;
    let mtm = MainThreadMarker::from(&*app);
    let (window, controller) = build_form(mtm, explanation, start_dir);

    window.makeKeyAndOrderFront(None);
    activate(&app);
    let answer = loop {
        if app.runModalForWindow(&window) != NSModalResponseOK {
            break None;
        }
        let form = controller.read();
        match validate(&form) {
            Ok(()) => break Some(form),
            // The alert runs its own modal on top; the window stays where it
            // is, with everything typed so far.
            Err(message) => error(app_name, &message),
        }
    };
    window.orderOut(None);
    answer
}

/// The window's controls, kept by the controller so the actions can reach
/// them and so they outlive the modal loop.
struct FormIvars {
    name: Retained<NSTextField>,
    executable: Retained<NSTextField>,
    arguments: Retained<NSTextField>,
    env: Retained<NSTextView>,
    save: Retained<NSButton>,
    /// Where the file picker starts when the executable field names nothing.
    start_dir: PathBuf,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, the class is only
    // created through `FormController::new`, and the ivars are plain Rust
    // values the host never touches.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = FormIvars]
    struct FormController;

    unsafe impl NSObjectProtocol for FormController {}

    impl FormController {
        #[unsafe(method(choose:))]
        fn choose(&self, _sender: Option<&NSButton>) {
            let ivars = self.ivars();
            let typed = PathBuf::from(ivars.executable.stringValue().to_string());
            let start = typed
                .parent()
                .filter(|dir| dir.is_dir())
                .map_or_else(|| ivars.start_dir.clone(), Path::to_path_buf);
            let app = NSApplication::sharedApplication(self.mtm());
            if let Some(picked) = open_panel(&app, &start) {
                ivars
                    .executable
                    .setStringValue(&NSString::from_str(&picked.to_string_lossy()));
            }
        }

        #[unsafe(method(run:))]
        fn run(&self, _sender: Option<&NSButton>) {
            NSApplication::sharedApplication(self.mtm()).stopModalWithCode(NSModalResponseOK);
        }

        #[unsafe(method(cancel:))]
        fn cancel(&self, _sender: Option<&NSButton>) {
            NSApplication::sharedApplication(self.mtm()).abortModal();
        }
    }
);

impl FormController {
    fn new(mtm: MainThreadMarker, ivars: FormIvars) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ivars);
        // SAFETY: plain NSObject init on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }

    fn read(&self) -> GameForm {
        let ivars = self.ivars();
        GameForm {
            name: ivars.name.stringValue().to_string(),
            executable: ivars.executable.stringValue().to_string(),
            arguments: ivars.arguments.stringValue().to_string(),
            env_text: ivars.env.string().to_string(),
            save: ivars.save.state() == NSControlStateValueOn,
        }
    }
}

const WIDTH: f64 = 580.0;
const MARGIN: f64 = 20.0;
const LABEL_WIDTH: f64 = 100.0;
const FIELD_X: f64 = MARGIN + LABEL_WIDTH + 10.0;
const FIELD_WIDTH: f64 = WIDTH - FIELD_X - MARGIN;
const INTRO_WIDTH: f64 = WIDTH - MARGIN - MARGIN;
const ROW: f64 = 22.0;
const GAP: f64 = 10.0;
const ENV_HEIGHT: f64 = 88.0;
const BUTTON_WIDTH: f64 = 90.0;
const BUTTON_HEIGHT: f64 = 32.0;

/// Build the window with its controls laid out from the top down; the window
/// is as tall as the explanation needs. Manual frames: the form is static, and
/// Auto Layout would cost more bindings than it saves here.
fn build_form(
    mtm: MainThreadMarker,
    explanation: &str,
    start_dir: &Path,
) -> (Retained<NSWindow>, Retained<FormController>) {
    let intro = NSTextField::wrappingLabelWithString(&NSString::from_str(explanation), mtm);
    intro.setPreferredMaxLayoutWidth(INTRO_WIDTH);
    let hint = NSTextField::wrappingLabelWithString(
        ns_string!("One NAME=value per line. Quote nothing: the value is everything after the =."),
        mtm,
    );
    hint.setPreferredMaxLayoutWidth(FIELD_WIDTH);

    let name = NSTextField::textFieldWithString(ns_string!(""), mtm);
    name.setPlaceholderString(Some(ns_string!(
        "Shown in the Dock menu; the executable's name when empty"
    )));
    let executable = NSTextField::textFieldWithString(ns_string!(""), mtm);
    executable.setPlaceholderString(Some(ns_string!("/path/to/Game.exe")));
    let arguments = NSTextField::textFieldWithString(ns_string!(""), mtm);
    arguments.setPlaceholderString(Some(ns_string!("-console \"two words\"")));

    let (env_scroll, env) = env_editor(mtm);

    // The checkbox has no action, so it can exist before the controller that
    // the buttons target.
    // SAFETY: no target and no action.
    let save = unsafe {
        NSButton::checkboxWithTitle_target_action(
            ns_string!("Save as new game (unchecked: run once, save nothing)"),
            None,
            None,
            mtm,
        )
    };
    save.setState(NSControlStateValueOn);

    let controller = FormController::new(
        mtm,
        FormIvars {
            name: name.clone(),
            executable: executable.clone(),
            arguments: arguments.clone(),
            env,
            save: save.clone(),
            start_dir: start_dir.to_path_buf(),
        },
    );
    let button = |title: &NSString, action: objc2::runtime::Sel| {
        let target: &AnyObject = &controller;
        // SAFETY: the selectors name the methods defined on FormController
        // above, each taking the sender as its one argument, and the
        // controller is returned alongside the window so it outlives the
        // controls.
        unsafe { NSButton::buttonWithTitle_target_action(title, Some(target), Some(action), mtm) }
    };
    let choose = button(ns_string!("Choose\u{2026}"), sel!(choose:));
    let cancel = button(ns_string!("Cancel"), sel!(cancel:));
    let run = button(ns_string!("Run"), sel!(run:));
    cancel.setKeyEquivalent(ns_string!("\u{1b}"));
    run.setKeyEquivalent(ns_string!("\r"));

    let mut layout = Layout::new(mtm);
    layout.place(
        &intro,
        MARGIN,
        INTRO_WIDTH,
        intro.fittingSize().height.ceil(),
    );
    layout.advance(GAP);
    layout.labelled("Name:", &name, FIELD_WIDTH, ROW);
    layout.labelled(
        "Executable:",
        &executable,
        FIELD_WIDTH - BUTTON_WIDTH - GAP,
        ROW,
    );
    layout.top -= ROW + GAP;
    layout.place_at(
        &choose,
        WIDTH - MARGIN - BUTTON_WIDTH,
        layout.top - 5.0,
        BUTTON_WIDTH,
        BUTTON_HEIGHT,
    );
    layout.advance(ROW + GAP);
    layout.labelled("Arguments:", &arguments, FIELD_WIDTH, ROW);
    layout.labelled("Environment:", &env_scroll, FIELD_WIDTH, ENV_HEIGHT);
    layout.top -= GAP - 4.0;
    layout.place(
        &hint,
        FIELD_X,
        FIELD_WIDTH,
        hint.fittingSize().height.ceil(),
    );
    layout.place(&save, FIELD_X, FIELD_WIDTH, ROW);
    layout.advance(GAP);
    let buttons_top = layout.top;
    layout.place_at(
        &cancel,
        WIDTH - MARGIN - BUTTON_WIDTH - GAP - BUTTON_WIDTH,
        buttons_top,
        BUTTON_WIDTH,
        BUTTON_HEIGHT,
    );
    layout.place_at(
        &run,
        WIDTH - MARGIN - BUTTON_WIDTH,
        buttons_top,
        BUTTON_WIDTH,
        BUTTON_HEIGHT,
    );
    layout.advance(BUTTON_HEIGHT + MARGIN);

    let window = layout.into_window(mtm, ns_string!("New Executable"));
    window.setInitialFirstResponder(Some(&executable));
    (window, controller)
}

/// A plain-text editor in a scroll view: the view to place and the text view
/// to read. macOS would otherwise prettify quotes and dashes as they are typed,
/// which is wrong for values that are passed on as they are.
fn env_editor(mtm: MainThreadMarker) -> (Retained<NSScrollView>, Retained<NSTextView>) {
    let scroll = NSTextView::scrollableTextView(mtm);
    let text = scroll
        .documentView()
        .and_then(|view| view.downcast::<NSTextView>().ok())
        .expect("scrollableTextView holds an NSTextView");
    text.setRichText(false);
    text.setAutomaticQuoteSubstitutionEnabled(false);
    text.setAutomaticDashSubstitutionEnabled(false);
    text.setAutomaticTextReplacementEnabled(false);
    (scroll, text)
}

/// Rows measured from the top of the window, turned into `AppKit` frames
/// (origin bottom left) once the total height is known.
struct Layout {
    mtm: MainThreadMarker,
    placed: Vec<(Retained<NSView>, f64, f64, f64, f64)>,
    /// Where the next row starts.
    top: f64,
}

impl Layout {
    const fn new(mtm: MainThreadMarker) -> Self {
        Self {
            mtm,
            placed: Vec::new(),
            top: MARGIN,
        }
    }

    fn advance(&mut self, by: f64) {
        self.top += by;
    }

    /// A view at the next row, which then moves past it.
    fn place(&mut self, view: &NSView, x: f64, width: f64, height: f64) {
        self.place_at(view, x, self.top, width, height);
        self.advance(height + GAP);
    }

    fn place_at(&mut self, view: &NSView, x: f64, top: f64, width: f64, height: f64) {
        self.placed.push((view.retain(), x, top, width, height));
    }

    /// A right-aligned label in the left column and the control next to it.
    fn labelled(&mut self, label: &str, control: &NSView, width: f64, height: f64) {
        let label = NSTextField::labelWithString(&NSString::from_str(label), self.mtm);
        label.setAlignment(NSTextAlignment::Right);
        self.place_at(&label, MARGIN, self.top + 1.0, LABEL_WIDTH, ROW);
        self.place(control, FIELD_X, width, height);
    }

    fn into_window(self, mtm: MainThreadMarker, title: &NSString) -> Retained<NSWindow> {
        let height = self.top;
        let content = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, height));
        // SAFETY: a titled window with deferred creation off; the `Retained`
        // owns it, so it must not release itself when closed.
        let window = unsafe {
            let window = NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                content,
                NSWindowStyleMask::Titled,
                NSBackingStoreType::Buffered,
                false,
            );
            window.setReleasedWhenClosed(false);
            window
        };
        window.setTitle(title);
        let content_view = window
            .contentView()
            .expect("a new window has a content view");
        for (view, x, top, width, h) in self.placed {
            view.setFrame(NSRect::new(
                NSPoint::new(x, height - top - h),
                NSSize::new(width, h),
            ));
            content_view.addSubview(&view);
        }
        window.center();
        window
    }
}

/// Check in with the window server. Returns `None` off the main thread, where
/// none of this is allowed.
fn become_app() -> Option<Retained<NSApplication>> {
    let mtm = MainThreadMarker::new()?;
    USED_GUI.store(true, Ordering::Relaxed);
    let app = NSApplication::sharedApplication(mtm);
    // Already the case when launched from Finder; this is for a terminal run,
    // where the dialog would otherwise have no way to take focus.
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    app.finishLaunching();
    Some(app)
}

/// Check in with `AppKit` without putting anything on screen. The updater
/// needs this split: Sparkle tells us it is about to show a window before the
/// window exists, and checking in has to happen first so the app does not
/// count as still starting up while that window is drawn.
pub fn check_in_as_app() {
    let _ = become_app();
}

/// Take focus. Like `activate`, only call this once a window is on screen.
pub fn activate_app() {
    if let Some(mtm) = MainThreadMarker::new() {
        activate(&NSApplication::sharedApplication(mtm));
    }
}

/// Call once the dialog's window is already on screen, never before.
fn activate(app: &NSApplication) {
    // Not `activate()`: that selector is macOS 14 and up, and the deployment
    // target here is 12, where calling it would be an unrecognized selector.
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
}
