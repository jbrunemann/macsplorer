//! Drag and drop with other apps, through AppKit: files dropped on the
//! window from Finder (or another Macsplorer window), and items dragged out
//! of it. Inside the window drags are the app's own (`item_drag`); one that
//! leaves the window is handed to AppKit here, so Finder, the Dock's Trash
//! and other apps can take it.
//!
//! winit registers the window for file drops but reports neither where the
//! pointer is nor the modifier keys, and always answers "copy". So the
//! main window's view gets the drop methods itself (added to winit's view
//! class, which has none) and registers for file URLs, which makes AppKit
//! ask it instead of winit's window delegate.
//!
//! The answer to "what would a drop here do" comes from the app's last
//! frame: AppKit asks again as the pointer moves, so it is at most a frame
//! behind.
//!
//! Only macOS has this; elsewhere nothing comes in and `begin` refuses.

use std::path::PathBuf;

use eframe::egui;
use macsplorer_core::dragdrop::Allowed;

/// A drag from outside the window (or one of ours that came back) while it
/// is over the window.
#[derive(Clone, Debug)]
pub struct Incoming {
    pub paths: Vec<PathBuf>,
    /// The effects the source offers, already narrowed by AppKit to what the
    /// modifier keys ask for (Option copies, Command moves, both link).
    pub allowed: Allowed,
    /// The pointer, in egui points.
    pub pos: egui::Pos2,
    /// The button went up and the drop was accepted: carry it out.
    pub dropped: bool,
}

#[cfg(target_os = "macos")]
pub use mac::{begin, incoming, install, set_answer};

#[cfg(not(target_os = "macos"))]
pub fn incoming() -> Option<Incoming> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn set_answer(_: Option<macsplorer_core::dragdrop::Effect>) {}

#[cfg(not(target_os = "macos"))]
pub fn begin(_: &egui::Context, _: &[PathBuf]) -> bool {
    false
}

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;
    use std::path::PathBuf;

    use eframe::egui;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject, Sel};
    use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_app_kit::{
        NSApplication, NSCursor, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession, NSDraggingSource, NSEvent, NSEventModifierFlags, NSEventType,
        NSPasteboard, NSPasteboardTypeFileURL, NSPasteboardWriting, NSView, NSWorkspace,
    };
    use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString, NSURL};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use macsplorer_core::dragdrop::{Allowed, Effect};

    use super::Incoming;

    #[derive(Default)]
    struct State {
        view: Option<Retained<NSView>>,
        ctx: Option<egui::Context>,
        incoming: Option<Incoming>,
        /// What a drop where the pointer is would do, from the last frame.
        answer: Option<Effect>,
        /// The session of a drag this window handed to AppKit, kept alive
        /// with its source until it ends.
        outgoing: Option<(Retained<NSDraggingSession>, Retained<DragSource>)>,
    }

    thread_local! {
        // AppKit calls in on the main thread, where the app runs too.
        static STATE: RefCell<State> = RefCell::default();
    }

    /// Makes the main window's view a drop target. Call once, from the
    /// creation closure, on the main thread.
    pub fn install(cc: &eframe::CreationContext<'_>) {
        let Ok(handle) = cc.window_handle() else { return };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else { return };
        // SAFETY: the handle's view is winit's live NSView for this window.
        let Some(view) = (unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>()) }) else { return };
        let class = view.class();
        let methods: [(Sel, objc2::runtime::Imp, &std::ffi::CStr); 4] = [
            // SAFETY (all four): the functions have the signature of the
            // selector they're added for, as the type strings say.
            (sel!(draggingEntered:), unsafe { std::mem::transmute::<OperationFn, objc2::runtime::Imp>(dragging_updated) }, c"Q@:@"),
            (sel!(draggingUpdated:), unsafe { std::mem::transmute::<OperationFn, objc2::runtime::Imp>(dragging_updated) }, c"Q@:@"),
            (sel!(draggingExited:), unsafe { std::mem::transmute::<VoidFn, objc2::runtime::Imp>(dragging_exited) }, c"v@:@"),
            (sel!(performDragOperation:), unsafe { std::mem::transmute::<BoolFn, objc2::runtime::Imp>(perform_drag) }, c"B@:@"),
        ];
        for (sel, imp, types) in methods {
            // SAFETY: winit's view class doesn't define these; adding them
            // doesn't change anything it does.
            unsafe { objc2::ffi::class_addMethod(class as *const AnyClass as *mut AnyClass, sel, imp, types.as_ptr()) };
        }
        // SAFETY: a constant AppKit initializes before any code runs.
        view.registerForDraggedTypes(&NSArray::from_slice(&[unsafe { NSPasteboardTypeFileURL }]));
        let ctx = cc.egui_ctx.clone();
        STATE.with_borrow_mut(|s| {
            s.view = Some(view);
            s.ctx = Some(ctx);
        });
    }

    /// The drag over the window, if any.
    pub fn incoming() -> Option<Incoming> {
        STATE.with_borrow(|s| s.incoming.clone())
    }

    /// What a drop at the pointer would do, which the next question gets.
    /// After a drop, call it with `None` to forget the drag.
    pub fn set_answer(effect: Option<Effect>) {
        STATE.with_borrow_mut(|s| {
            s.answer = effect;
            if s.incoming.as_ref().is_some_and(|i| i.dropped) {
                s.incoming = None;
            }
        });
    }

    type OperationFn = unsafe extern "C-unwind" fn(&NSView, Sel, &AnyObject) -> usize;
    type VoidFn = unsafe extern "C-unwind" fn(&NSView, Sel, &AnyObject);
    type BoolFn = unsafe extern "C-unwind" fn(&NSView, Sel, &AnyObject) -> Bool;

    /// `draggingEntered:` and `draggingUpdated:`: notes the drag and where it
    /// is, and answers with the effect from the last frame.
    unsafe extern "C-unwind" fn dragging_updated(view: &NSView, _: Sel, sender: &AnyObject) -> usize {
        // SAFETY: `sender` is the NSDraggingInfo AppKit passes.
        let (location, mask, pasteboard): (NSPoint, NSDragOperation, Retained<NSPasteboard>) =
            unsafe { (msg_send![sender, draggingLocation], msg_send![sender, draggingSourceOperationMask], msg_send![sender, draggingPasteboard]) };
        let local = view.convertPoint_fromView(location, None);
        let y = if view.isFlipped() { local.y } else { view.bounds().size.height - local.y };
        STATE.with_borrow_mut(|s| {
            let zoom = s.ctx.as_ref().map_or(1.0, |c| c.zoom_factor());
            let pos = egui::pos2(local.x as f32 / zoom, y as f32 / zoom);
            let allowed = allowed(mask);
            match &mut s.incoming {
                Some(i) => (i.pos, i.allowed) = (pos, allowed),
                None => {
                    s.answer = None;
                    s.incoming = Some(Incoming { paths: crate::pasteboard::files_on(&pasteboard), allowed, pos, dropped: false });
                }
            }
            if let Some(ctx) = &s.ctx {
                ctx.request_repaint();
            }
            operation(s.answer, mask).0
        })
    }

    unsafe extern "C-unwind" fn dragging_exited(_: &NSView, _: Sel, _: &AnyObject) {
        STATE.with_borrow_mut(|s| {
            s.incoming = None;
            s.answer = None;
            if let Some(ctx) = &s.ctx {
                ctx.request_repaint();
            }
        });
    }

    /// Accepts the drop if the last answer did; the app carries it out on
    /// its next frame.
    unsafe extern "C-unwind" fn perform_drag(_: &NSView, _: Sel, _: &AnyObject) -> Bool {
        STATE.with_borrow_mut(|s| {
            let accept = s.answer.is_some() && s.incoming.is_some();
            match &mut s.incoming {
                Some(i) if accept => i.dropped = true,
                _ => s.incoming = None,
            }
            if let Some(ctx) = &s.ctx {
                ctx.request_repaint();
            }
            Bool::new(accept)
        })
    }

    /// The effects in an operation mask. Command makes AppKit offer only
    /// "generic", which Finder treats as a move.
    fn allowed(mask: NSDragOperation) -> Allowed {
        let mut effects = Vec::new();
        if mask.contains(NSDragOperation::Copy) {
            effects.push(Effect::Copy);
        }
        if mask.intersects(NSDragOperation::Move | NSDragOperation::Generic) {
            effects.push(Effect::Move);
        }
        if mask.contains(NSDragOperation::Link) {
            effects.push(Effect::Link);
        }
        Allowed::of(&effects)
    }

    /// The operation for an effect, one of those in `mask`.
    fn operation(effect: Option<Effect>, mask: NSDragOperation) -> NSDragOperation {
        match effect {
            None => NSDragOperation::None,
            Some(Effect::Copy) => NSDragOperation::Copy,
            Some(Effect::Link) => NSDragOperation::Link,
            Some(Effect::Move) if mask.contains(NSDragOperation::Move) => NSDragOperation::Move,
            Some(Effect::Move) => NSDragOperation::Generic,
        }
    }

    define_class!(
        /// The source of a drag handed to AppKit.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "MacsplorerDragSource"]
        struct DragSource;

        unsafe impl NSObjectProtocol for DragSource {}

        unsafe impl NSDraggingSource for DragSource {
            /// Everything, inside the app and out: the target decides, as
            /// Finder does for its own drags (moving on the same volume,
            /// copying to another, deleting on the Trash).
            #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
            fn source_mask(&self, _session: &NSDraggingSession, _context: NSDraggingContext) -> NSDragOperation {
                NSDragOperation::Copy | NSDragOperation::Move | NSDragOperation::Link | NSDragOperation::Generic | NSDragOperation::Delete
            }

            /// AppKit kept the button-up to itself; give winit one, so egui
            /// doesn't think the button is still down.
            #[unsafe(method(draggingSession:endedAtPoint:operation:))]
            fn ended(&self, _session: &NSDraggingSession, _point: NSPoint, _operation: NSDragOperation) {
                let mtm = self.mtm();
                let (view, ctx) = STATE.with_borrow(|s| (s.view.clone(), s.ctx.clone()));
                {
                    if let Some(window) = view.as_ref().and_then(|v| v.window()) {
                        let up = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                            NSEventType::LeftMouseUp,
                            window.mouseLocationOutsideOfEventStream(),
                            NSEventModifierFlags::empty(),
                            0.0,
                            window.windowNumber(),
                            None,
                            0,
                            1,
                            0.0,
                        );
                        if let Some(up) = up {
                            NSApplication::sharedApplication(mtm).postEvent_atStart(&up, false);
                        }
                    }
                    if let Some(ctx) = &ctx {
                        ctx.request_repaint();
                    }
                }
                // Dropped last: this is the session's source.
                let outgoing = STATE.with_borrow_mut(|s| s.outgoing.take());
                drop(outgoing);
            }
        }
    );

    impl DragSource {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            // SAFETY: NSObject's init.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// Hands a drag of `paths` to AppKit, the system's icons as its image.
    /// Returns false when it couldn't start.
    pub fn begin(_ctx: &egui::Context, paths: &[PathBuf]) -> bool {
        let Some(mtm) = MainThreadMarker::new() else { return false };
        // Not borrowed while AppKit runs, which may call the view back.
        let Some(view) = STATE.with_borrow(|s| s.view.clone()) else { return false };
        {
            let Some(window) = view.window() else { return false };
            let at = view.convertPoint_fromView(window.mouseLocationOutsideOfEventStream(), None);
            let workspace = NSWorkspace::sharedWorkspace();
            let mut items = Vec::new();
            for (i, path) in paths.iter().enumerate() {
                let Some(text) = path.to_str() else { continue };
                let text = NSString::from_str(text);
                let url = NSURL::fileURLWithPath(&text);
                let item = NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), ProtocolObject::<dyn NSPasteboardWriting>::from_ref(&*url));
                // A small stack of icons under the pointer.
                let step = i.min(4) as f64 * 4.0;
                let frame = NSRect::new(NSPoint::new(at.x - 16.0 + step, at.y - 16.0 - step), NSSize::new(32.0, 32.0));
                let icon = workspace.iconForFile(&text);
                // SAFETY: an NSImage is valid contents for a dragging frame.
                unsafe { item.setDraggingFrame_contents(frame, Some(&icon)) };
                items.push(item);
            }
            if items.is_empty() {
                return false;
            }
            let app = NSApplication::sharedApplication(mtm);
            let event = app
                .currentEvent()
                .filter(|e| matches!(e.r#type(), NSEventType::LeftMouseDragged | NSEventType::LeftMouseDown))
                .or_else(|| {
                    NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                        NSEventType::LeftMouseDragged,
                        window.mouseLocationOutsideOfEventStream(),
                        NSEventModifierFlags::empty(),
                        0.0,
                        window.windowNumber(),
                        None,
                        0,
                        1,
                        1.0,
                    )
                });
            let Some(event) = event else { return false };
            // The window's last cursor (often the "no" sign: the items can't
            // go back into their own folder) would stay up over other apps
            // until one of them sets its own, since winit only sets cursors
            // inside the window. Start from the arrow, as Finder's drags do.
            NSCursor::arrowCursor().set();
            let source = DragSource::new(mtm);
            let session = view.beginDraggingSessionWithItems_event_source(&NSArray::from_retained_slice(&items), &event, ProtocolObject::from_ref(&*source));
            STATE.with_borrow_mut(|s| s.outgoing = Some((session, source)));
            true
        }
    }
}
