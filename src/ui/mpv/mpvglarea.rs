use glib::Object;
use gtk::{
    gdk::{
        Backend,
        Display,
    },
    gio,
    glib,
    prelude::*,
    subclass::prelude::*,
};
use libmpv2::SetData;
use tracing::info;

use super::tsukimi_mpv::{
    TrackSelection,
    ACTIVE,
};
use crate::client::emby_client::EMBY_CLIENT;

mod imp {
    use std::thread::JoinHandle;

    use gettextrs::gettext;
    use gtk::{
        gdk::GLContext,
        glib,
        prelude::*,
        subclass::prelude::*,
    };
    use once_cell::sync::OnceCell;

    use crate::{
        close_on_error,
        ui::mpv::tsukimi_mpv::{
            TsukimiMPV,
            RENDER_UPDATE,
        },
    };

    // Object holding the state
    #[derive(Default)]
    pub struct MPVGLArea {
        pub mpv: TsukimiMPV,
        pub mpv_event_loop: OnceCell<JoinHandle<()>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MPVGLArea {
        const NAME: &'static str = "MPVGLArea";
        type Type = super::MPVGLArea;
        type ParentType = gtk::GLArea;
    }

    impl ObjectImpl for MPVGLArea {
        fn constructed(&self) {
            self.parent_constructed();
        }

        fn dispose(&self) {
            if let Ok(mpv) = self.mpv.mpv.lock() {
                drop(mpv);
            }
        }
    }

    impl WidgetImpl for MPVGLArea {
        fn realize(&self) {
            self.parent_realize();
            let obj = self.obj();
            obj.make_current();
            let Some(gl_context) = self.obj().context() else {
                close_on_error!(self.obj(), gettext("Failed to get GLContext"));
                return;
            };

            self.mpv.connect_render_update(gl_context);

            glib::spawn_future_local(glib::clone!(
                #[weak]
                obj,
                async move {
                    while let Ok(true) = RENDER_UPDATE.rx.recv_async().await {
                        obj.queue_render();
                    }
                }
            ));

            self.mpv.process_events();
        }
    }

    impl GLAreaImpl for MPVGLArea {
        fn render(&self, _context: &GLContext) -> glib::Propagation {
            let binding = self.mpv.ctx.borrow();
            let Some(ctx) = binding.as_ref() else {
                return glib::Propagation::Stop;
            };

            let factor = self.obj().scale_factor();
            let width = self.obj().width() * factor;
            let height = self.obj().height() * factor;
            unsafe {
                let mut fbo = -1;
                gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut fbo);
                ctx.render::<GLContext>(fbo, width, height, true).unwrap();
            }
            glib::Propagation::Stop
        }
    }
}

glib::wrapper! {
    pub struct MPVGLArea(ObjectSubclass<imp::MPVGLArea>)
        @extends gtk::ApplicationWindow, gtk::Window, gtk::Widget ,gtk::GLArea,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl Default for MPVGLArea {
    fn default() -> Self {
        Self::new()
    }
}

impl MPVGLArea {
    pub fn new() -> Self {
        Object::builder().build()
    }

    pub fn play(&self, url: &str, percentage: f64) {
        let mpv = &self.imp().mpv;

        mpv.event_thread_alive
            .store(ACTIVE, std::sync::atomic::Ordering::SeqCst);
        atomic_wait::wake_all(&*mpv.event_thread_alive);

        let url = EMBY_CLIENT.get_streaming_url(url);

        info!("Now Playing: {}", url);
        mpv.load_video(&url);

        mpv.set_start(percentage);

        mpv.pause(false);
    }

    pub fn add_sub(&self, url: &str) {
        self.imp().mpv.add_sub(url)
    }

    pub fn seek_forward(&self, value: i64) {
        self.imp().mpv.seek_forward(value)
    }

    pub fn seek_backward(&self, value: i64) {
        self.imp().mpv.seek_backward(value)
    }

    pub fn set_position(&self, value: f64) {
        self.imp().mpv.set_position(value)
    }

    pub fn position(&self) -> f64 {
        self.imp().mpv.position()
    }

    pub fn get_wid(&self) -> Option<u64> {
        return None;

        // FIXME: x11 and win32 display
        #[allow(unreachable_code)]
        match Display::default()?.backend() {
            Backend::X11 => {
                #[cfg(target_os = "linux")]
                {
                    self.native()?
                        .surface()
                        .and_downcast_ref::<gdk4_x11::X11Surface>()
                        .map(|s| s.xid())
                }

                #[cfg(not(target_os = "linux"))]
                {
                    None
                }
            }
            Backend::Win32 => {
                #[cfg(target_os = "windows")]
                {
                    self.native()?
                        .surface()
                        .and_downcast_ref::<gdk4_win32::Win32Surface>()
                        .map(|s| s.handle().0 as u64)
                }

                #[cfg(not(target_os = "windows"))]
                {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn set_aid(&self, value: TrackSelection) {
        self.imp().mpv.set_aid(value)
    }

    pub fn get_track_id(&self, type_: &str) -> i64 {
        self.imp().mpv.get_track_id(type_)
    }

    pub fn set_sid(&self, value: TrackSelection) {
        self.imp().mpv.set_sid(value)
    }

    pub fn press_key(&self, key: u32, state: gtk::gdk::ModifierType) {
        self.imp().mpv.press_key(key, state)
    }

    pub fn release_key(&self, key: u32, state: gtk::gdk::ModifierType) {
        self.imp().mpv.release_key(key, state)
    }

    pub fn set_speed(&self, value: f64) {
        self.imp().mpv.set_speed(value)
    }

    pub fn set_volume(&self, value: i64) {
        self.imp().mpv.set_volume(value)
    }

    pub fn display_stats_toggle(&self) {
        self.imp().mpv.display_stats_toggle()
    }

    pub fn paused(&self) -> bool {
        self.imp().mpv.paused()
    }

    pub fn pause(&self) {
        self.imp().mpv.command_pause();
    }

    pub fn set_property<V>(&self, property: &str, value: V)
    where
        V: SetData + Send + 'static,
    {
        self.imp().mpv.set_property(property, value)
    }
}
