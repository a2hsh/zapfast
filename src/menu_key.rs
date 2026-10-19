//! The Applications key (the context menu key) on Windows, which opens the
//! focused list item's menu as Shift+F10 does.
//!
//! egui has no key for it, so the key winit reports never reaches the app.
//! Nor does Windows turn it into `WM_CONTEXTMENU`: that comes from
//! `DefWindowProc` handling the key's own messages, and winit's window
//! procedure answers `WM_KEYDOWN` and `WM_KEYUP` itself. A subclass of the
//! window sees the key's `WM_KEYDOWN` first and hands it to the next frame as
//! a Shift+F10 press, so every list that opens a menu from Shift+F10 opens it
//! from this key too, and nothing else does.
//!
//! The same subclass keeps Alt with a letter or a digit, such as the Alt+1,
//! Alt+2 and Alt+3 shortcuts, out of the system menu. winit hands their
//! `WM_SYSCHAR` to `DefWindowProc`, which looks the character up as a menu
//! mnemonic: it starts the system menu's keyboard tracking and ends it at
//! once, as the window has no menu bar to find the character in. A screen
//! reader hears that menu start and end, and reads the focus again.

use std::cell::Cell;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_APPS, VK_CONTROL, VK_LWIN, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    SC_KEYMENU, WM_CONTEXTMENU, WM_KEYDOWN, WM_NCDESTROY, WM_SYSCOMMAND,
};

/// `WM_KEYDOWN`'s previous key state: set on the repeats of a held key.
const REPEAT: isize = 1 << 30;

const SUBCLASS_ID: usize = 0x5a46_4d4b;

thread_local! {
    /// The window whose Applications key was pressed since the last frame,
    /// set by the subclass and taken by that window's next frame. Both run
    /// on the thread that owns the window.
    static PRESSED: Cell<isize> = const { Cell::new(0) };
}

/// One window's Applications key.
#[derive(Default)]
pub struct MenuKey {
    /// The window watched, once its subclass is installed.
    window: Option<isize>,
    /// Whether the subclass has been installed, or failed to be; it is
    /// tried once per window, as a recreated window comes with a new
    /// `MenuKey`.
    tried: bool,
}

impl MenuKey {
    /// Starts watching `window` for the key. Called every frame: a frame
    /// without the window's handle tries again on the next, and once there
    /// is a handle the subclass is tried only once.
    pub fn watch(&mut self, window: &impl HasWindowHandle) {
        if self.tried {
            return;
        }
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return;
        };
        self.tried = true;
        let hwnd = handle.hwnd.get();
        // SAFETY: this runs on the window thread; the subclass removes
        // itself on WM_NCDESTROY.
        let installed = unsafe {
            SetWindowSubclass(
                HWND(hwnd as *mut std::ffi::c_void),
                Some(subclass),
                SUBCLASS_ID,
                0,
            )
            .as_bool()
        };
        if installed {
            self.window = Some(hwnd);
        } else {
            log::debug!("could not watch for the Applications key");
        }
    }

    /// Adds a Shift+F10 press to this frame's `input` when the key was
    /// pressed in the watched window since the last frame.
    pub fn deliver(&mut self, input: &mut egui::RawInput) {
        let Some(window) = self.window else {
            return;
        };
        let pressed = PRESSED.with(|pressed| {
            let here = pressed.get() == window;
            if here {
                pressed.set(0);
            }
            here
        });
        if pressed {
            input.events.extend(shift_f10());
        }
    }

    /// A `MenuKey` that takes the presses recorded for `window`, without
    /// a real window to subclass.
    #[cfg(test)]
    pub(crate) fn watching(window: isize) -> Self {
        Self {
            window: Some(window),
            tried: true,
        }
    }
}

/// Records a press of the key in `window`, as the subclass does.
pub(crate) fn note_press(window: isize) {
    PRESSED.with(|pressed| pressed.set(window));
}

/// A Shift+F10 press, the keyboard's other way to open a menu, and its
/// release, so egui does not count F10 as held down.
fn shift_f10() -> [egui::Event; 2] {
    let key = |pressed: bool| egui::Event::Key {
        key: egui::Key::F10,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::SHIFT,
    };
    [key(true), key(false)]
}

/// What the subclass does with a window message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handling {
    /// The Applications key went down on its own: record it, then pass the
    /// message on.
    Press,
    /// Answer it here: a keyboard `WM_CONTEXTMENU`, or the system menu's
    /// mnemonic lookup for Alt with a character.
    Swallow,
    /// Pass it on untouched.
    Pass,
}

/// Sorts a window message. Each key opens a menu from one message only:
/// the Applications key from its first `WM_KEYDOWN` (not its repeats), and
/// Shift+F10 from egui's own key event. The Applications key counts on its
/// own only: `modified` says whether Ctrl, Shift or a Windows key was held,
/// and is asked for that key alone. Windows gives those combinations no
/// meaning (Shift is some apps' extended menu, which ZapFast has none of),
/// and Ctrl+Shift+F10 opens nothing either. With Alt, the key is a system
/// key and never a `WM_KEYDOWN`. A `WM_CONTEXTMENU` from the keyboard
/// (both coordinates -1) only follows one of those keys, so it is answered
/// here and never opens a second menu. Shift+F10 sends one today: F10 is a
/// system key, whose `WM_SYSKEYDOWN` winit hands to `DefWindowProc`. Passed
/// on, `DefWindowProc` would hit-test the point (-1, -1), which shows the
/// system menu when the window covers the screen's top-left corner. A
/// `WM_CONTEXTMENU` from the pointer carries a real point and passes on, so
/// right-clicks stay as they are.
///
/// `SC_KEYMENU` with a character is `DefWindowProc` looking up Alt with that
/// character (Alt+2, Alt+F) as a menu mnemonic. With no menu bar it never
/// finds one, so answering it here changes nothing but the menu start and
/// end a screen reader would hear. The character itself reached winit, and
/// egui, from `WM_SYSCHAR` before this. `SC_KEYMENU` with a space (Alt+Space,
/// the system menu) or with none (Alt or F10 alone) passes on as before, and
/// so does every other system command, such as Alt+F4's `SC_CLOSE`.
fn handling(
    message: u32,
    wparam: usize,
    lparam: isize,
    modified: impl FnOnce() -> bool,
) -> Handling {
    match message {
        WM_KEYDOWN if wparam == usize::from(VK_APPS.0) && lparam & REPEAT == 0 && !modified() => {
            Handling::Press
        }
        WM_CONTEXTMENU if from_keyboard(lparam) => Handling::Swallow,
        WM_SYSCOMMAND if is_mnemonic(wparam, lparam) => Handling::Swallow,
        _ => Handling::Pass,
    }
}

/// Whether a `WM_CONTEXTMENU` came from the keyboard: its point is
/// `MAKELPARAM(-1, -1)`, both 16-bit screen coordinates -1. Only the low 32
/// bits are read, as Windows may widen them with or without the sign. A
/// right-click at exactly that screen point, on a title bar left of and
/// above the main screen, looks the same; Windows offers no way to tell
/// them apart, and the keyboard reading is the one that matters here.
fn from_keyboard(lparam: isize) -> bool {
    lparam as u32 == u32::MAX
}

/// Whether a `WM_SYSCOMMAND` is `SC_KEYMENU` for Alt with a character other
/// than a space. The low four bits of `wparam` are Windows' own.
fn is_mnemonic(wparam: usize, lparam: isize) -> bool {
    wparam & 0xFFF0 == SC_KEYMENU as usize && lparam != 0 && lparam != b' ' as isize
}

/// Whether Ctrl, Shift or a Windows key is down, as of the message being
/// handled.
fn modifier_held() -> bool {
    [VK_CONTROL, VK_SHIFT, VK_LWIN, VK_RWIN]
        .into_iter()
        .any(|key| {
            // SAFETY: GetKeyState only reads this thread's keyboard state.
            let state = unsafe { GetKeyState(i32::from(key.0)) };
            // The high bit: the key is down.
            state < 0
        })
}

unsafe extern "system" fn subclass(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    _reference_data: usize,
) -> LRESULT {
    match handling(message, wparam.0, lparam.0, modifier_held) {
        Handling::Press => {
            note_press(hwnd.0 as isize);
            // SAFETY: the live window repaints, and its next frame takes
            // the press.
            let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
        }
        Handling::Swallow => return LRESULT(0),
        Handling::Pass if message == WM_NCDESTROY => {
            // A press the window never took must not reach a later window
            // that Windows gives the same handle.
            PRESSED.with(|pressed| {
                if pressed.get() == hwnd.0 as isize {
                    pressed.set(0);
                }
            });
            // SAFETY: this removes our subclass before the window is destroyed.
            unsafe {
                let _ = RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID);
            };
        }
        Handling::Pass => {}
    }
    // SAFETY: every other message continues through the window's existing
    // procedure.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WM_KEYUP: u32 = 0x0101;
    const WM_CHAR: u32 = 0x0102;
    const WM_SYSKEYDOWN: u32 = 0x0104;
    const WM_SYSKEYUP: u32 = 0x0105;
    const WM_SYSCHAR: u32 = 0x0106;
    const SC_CLOSE: usize = 0xF060;
    const SC_MINIMIZE: usize = 0xF020;
    const VK_SHIFT: usize = 0x10;
    const VK_MENU: usize = 0x12;
    const VK_NUMPAD0: usize = 0x60;
    const VK_NUMPAD2: usize = 0x62;
    const VK_NUMPAD3: usize = 0x63;
    const VK_F10: usize = 0x79;
    /// A system key's lParam with Alt held (context code, bit 29).
    const ALT: isize = 1 | (1 << 29);
    /// The key's first `WM_KEYDOWN`: repeat count 1, previous state up.
    const FIRST: isize = 1;
    /// A held key's repeat.
    const HELD: isize = 1 | REPEAT;
    /// `MAKELPARAM(-1, -1)` as Windows may pass it on 64-bit: zero-extended.
    const KEYBOARD_POINT: isize = 0xFFFF_FFFF;
    const APPS: usize = VK_APPS.0 as usize;

    /// How the subclass sorts a message with no modifier held.
    fn plain(message: u32, wparam: usize, lparam: isize) -> Handling {
        handling(message, wparam, lparam, || false)
    }

    /// How many menus each message sequence opens through the subclass.
    fn presses(messages: &[(u32, usize, isize)]) -> usize {
        messages
            .iter()
            .filter(|(message, wparam, lparam)| {
                plain(*message, *wparam, *lparam) == Handling::Press
            })
            .count()
    }

    #[test]
    fn the_applications_key_is_one_press() {
        assert_eq!(plain(WM_KEYDOWN, APPS, FIRST), Handling::Press);
        // Held, it does not open the menu again on each repeat.
        assert_eq!(plain(WM_KEYDOWN, APPS, HELD), Handling::Pass);
        assert_eq!(plain(WM_KEYUP, APPS, FIRST), Handling::Pass);
        // Its messages as Windows sends them, with the WM_CONTEXTMENU a
        // procedure that hands key-ups to DefWindowProc would add: one menu.
        let key = [
            (WM_KEYDOWN, APPS, FIRST),
            (WM_KEYDOWN, APPS, HELD),
            (WM_KEYUP, APPS, FIRST),
            (WM_CONTEXTMENU, 0, KEYBOARD_POINT),
        ];
        assert_eq!(presses(&key), 1);
    }

    #[test]
    fn shift_f10_is_left_to_egui() {
        // Shift+F10's messages, with the WM_CONTEXTMENU DefWindowProc sends
        // for its WM_SYSKEYDOWN: the subclass opens nothing, egui's key
        // event does, so the menu opens once.
        let key = [
            (WM_KEYDOWN, VK_SHIFT, FIRST),
            (WM_SYSKEYDOWN, VK_F10, FIRST),
            (WM_CONTEXTMENU, 0, KEYBOARD_POINT),
            (WM_SYSKEYUP, VK_F10, FIRST),
            (WM_KEYUP, VK_SHIFT, FIRST),
        ];
        assert_eq!(presses(&key), 0);
        assert_eq!(
            plain(WM_SYSKEYDOWN, VK_F10, FIRST),
            Handling::Pass,
            "egui still sees F10"
        );
    }

    #[test]
    fn a_keyboard_context_menu_message_is_answered_here() {
        assert_eq!(plain(WM_CONTEXTMENU, 0, KEYBOARD_POINT), Handling::Swallow);
        // Sign-extended, as other versions pass it.
        assert_eq!(plain(WM_CONTEXTMENU, 0, -1), Handling::Swallow);
    }

    #[test]
    fn a_right_click_passes_on() {
        // A pointer WM_CONTEXTMENU carries the screen point: x 300, y 200.
        assert_eq!(plain(WM_CONTEXTMENU, 0, (200 << 16) | 300), Handling::Pass);
        // A point left of and above the main screen is negative, but not
        // both -1.
        let left_of_main = (((-40i16 as u16) as isize) << 16) | (-1i16 as u16) as isize;
        assert_eq!(plain(WM_CONTEXTMENU, 0, left_of_main), Handling::Pass);
    }

    #[test]
    fn other_keys_pass_on() {
        for key in [0x0D, 0x1B, 0x28, 0x79, 0x5B] {
            assert_eq!(plain(WM_KEYDOWN, key, FIRST), Handling::Pass);
        }
        // Alt with the key is a system key; Windows has no meaning for it.
        assert_eq!(plain(WM_SYSKEYDOWN, APPS, FIRST), Handling::Pass);
    }

    #[test]
    fn alt_with_a_character_skips_only_the_menu_lookup() {
        let keymenu = SC_KEYMENU as usize;
        // Alt+2 as Windows sends it: winit takes the key and its character,
        // and DefWindowProc's mnemonic lookup is the one message answered.
        let alt_2 = [
            (WM_SYSKEYDOWN, VK_MENU, ALT),
            (WM_SYSKEYDOWN, 0x32, ALT),
            (WM_SYSCHAR, 0x32, ALT),
            (WM_SYSCOMMAND, keymenu, 0x32),
            (WM_SYSKEYUP, 0x32, ALT),
            (WM_KEYUP, VK_MENU, FIRST),
        ];
        let handled: Vec<Handling> = alt_2
            .iter()
            .map(|(message, wparam, lparam)| plain(*message, *wparam, *lparam))
            .collect();
        let pass = Handling::Pass;
        assert_eq!(
            handled,
            [pass, pass, pass, Handling::Swallow, pass, pass],
            "Alt+2"
        );
        // Alt+1, Alt+3, a letter and a non-ASCII character alike, whatever
        // Windows puts in wParam's low bits.
        for character in ['1', '3', 'f', 'é'] {
            for wparam in [keymenu, keymenu | 0x2] {
                assert_eq!(
                    plain(WM_SYSCOMMAND, wparam, character as isize),
                    Handling::Swallow,
                    "Alt+{character}"
                );
            }
        }
    }

    #[test]
    fn alt_space_a_lone_alt_and_alt_f4_are_left_to_windows() {
        let keymenu = SC_KEYMENU as usize;
        // Alt+Space opens the system menu.
        assert_eq!(plain(WM_SYSCOMMAND, keymenu, ' ' as isize), Handling::Pass);
        // Alt or F10 alone: no character.
        assert_eq!(plain(WM_SYSCOMMAND, keymenu, 0), Handling::Pass);
        // Alt+F4, and the system menu's own commands, which carry the
        // pointer's position, not a character.
        assert_eq!(plain(WM_SYSCOMMAND, SC_CLOSE, 0), Handling::Pass);
        assert_eq!(
            plain(WM_SYSCOMMAND, SC_MINIMIZE, (200 << 16) | 300),
            Handling::Pass
        );
    }

    #[test]
    fn alt_numpad_codes_never_meet_the_menu_lookup() {
        // Alt+0233 (é): the numpad keys are system keys while Alt is held,
        // and the character arrives as WM_CHAR once Alt is let go. Windows
        // composes it without a WM_SYSCHAR, so no SC_KEYMENU follows, and
        // nothing in it is answered here.
        let mut code = vec![(WM_SYSKEYDOWN, VK_MENU, ALT)];
        for key in [VK_NUMPAD0, VK_NUMPAD2, VK_NUMPAD3, VK_NUMPAD3] {
            code.push((WM_SYSKEYDOWN, key, ALT));
            code.push((WM_SYSKEYUP, key, ALT));
        }
        code.push((WM_KEYUP, VK_MENU, FIRST));
        code.push((WM_CHAR, 0xE9, FIRST));
        for (message, wparam, lparam) in code {
            assert_eq!(plain(message, wparam, lparam), Handling::Pass);
        }
    }

    #[test]
    fn the_key_with_a_modifier_opens_nothing() {
        // Ctrl, Shift or a Windows key held as the key goes down.
        assert_eq!(handling(WM_KEYDOWN, APPS, FIRST, || true), Handling::Pass);
        // The modifiers are read for the Applications key alone, and only
        // when it goes down.
        let asked = std::cell::Cell::new(0);
        let ask = || {
            asked.set(asked.get() + 1);
            true
        };
        for (message, wparam, lparam) in [
            (WM_KEYDOWN, 0x41, FIRST),
            (WM_KEYDOWN, APPS, HELD),
            (WM_KEYUP, APPS, FIRST),
            (WM_CONTEXTMENU, 0, KEYBOARD_POINT),
        ] {
            handling(message, wparam, lparam, ask);
        }
        assert_eq!(asked.get(), 0);
    }

    #[test]
    fn a_press_reaches_only_its_own_window_once() {
        let mut first = MenuKey::watching(0x1000);
        let mut second = MenuKey::watching(0x2000);
        let mut unwatched = MenuKey::default();
        note_press(0x1000);
        let frame = |key: &mut MenuKey| {
            let mut input = egui::RawInput::default();
            key.deliver(&mut input);
            input.events
        };
        assert!(frame(&mut second).is_empty(), "another window");
        assert!(frame(&mut unwatched).is_empty(), "no window watched");
        assert_eq!(frame(&mut first), shift_f10());
        assert!(frame(&mut first).is_empty(), "taken once");
    }

    #[test]
    fn the_key_arrives_as_a_shift_f10_press_and_release() {
        let [press, release] = shift_f10();
        assert!(matches!(
            press,
            egui::Event::Key {
                key: egui::Key::F10,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::SHIFT,
                ..
            }
        ));
        assert!(matches!(
            release,
            egui::Event::Key {
                key: egui::Key::F10,
                pressed: false,
                ..
            }
        ));
    }
}
