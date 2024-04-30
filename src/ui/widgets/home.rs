use std::env;

use crate::client::{network::*, structs::*};
use crate::ui::provider::tu_item::TuItem;
use crate::utils::{
    get_data_with_cache, spawn, tu_list_item_factory, tu_list_view_connect_activate,
};
use adw::prelude::NavigationPageExt;
use glib::Object;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};

use super::tu_list_item::TuListItem;
use super::{fix::fix, list::ListPage, window::Window};

mod imp {

    use glib::subclass::InitializingObject;
    use gtk::subclass::prelude::*;
    use gtk::{glib, CompositeTemplate};

    use crate::utils::spawn_g_timeout;
    // Object holding the state
    #[derive(CompositeTemplate, Default)]
    #[template(resource = "/moe/tsukimi/home.ui")]
    pub struct HomePage {
        #[template_child]
        pub root: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        pub libscrolled: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        pub librevealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub liblist: TemplateChild<gtk::ListView>,
        #[template_child]
        pub libsbox: TemplateChild<gtk::Box>,
        #[template_child]
        pub toast: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub libsrevealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub spinner: TemplateChild<gtk::Spinner>,
        pub selection: gtk::SingleSelection,
    }

    // The central trait for subclassing a GObject
    #[glib::object_subclass]
    impl ObjectSubclass for HomePage {
        // `NAME` needs to match `class` attribute of template
        const NAME: &'static str = "HomePage";
        type Type = super::HomePage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    // Trait shared by all GObjects
    impl ObjectImpl for HomePage {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            spawn_g_timeout(glib::clone!(@weak obj => async move {
                obj.set_library().await;
            }));
        }
    }

    // Trait shared by all widgets
    impl WidgetImpl for HomePage {}

    // Trait shared by all windows
    impl WindowImpl for HomePage {}

    // Trait shared by all application windows
    impl ApplicationWindowImpl for HomePage {}

    impl adw::subclass::navigation_page::NavigationPageImpl for HomePage {}
}

glib::wrapper! {
    pub struct HomePage(ObjectSubclass<imp::HomePage>)
        @extends gtk::ApplicationWindow, gtk::Window, gtk::Widget ,adw::NavigationPage,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl Default for HomePage {
    fn default() -> Self {
        Self::new()
    }
}

impl HomePage {
    pub fn new() -> Self {
        Object::builder().build()
    }

    pub async fn set_library(&self) {
        self.set_libraryscorll().await;
    }

    pub async fn set_libraryscorll(&self) {
        let imp = self.imp();
        let libscrolled = fix(imp.libscrolled.get());
        imp.librevealer.set_reveal_child(true);
        let store = gtk::gio::ListStore::new::<glib::BoxedAnyObject>();
        imp.selection.set_autoselect(false);
        imp.selection.set_model(Some(&store));
        let selection = &imp.selection;
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_bind(move |_, item| {
            let list_item = item
                .downcast_ref::<gtk::ListItem>()
                .expect("Needs to be ListItem");
            if list_item.child().is_none() {
                let entry = item
                    .downcast_ref::<gtk::ListItem>()
                    .expect("Needs to be ListItem")
                    .item()
                    .and_downcast::<glib::BoxedAnyObject>()
                    .expect("Needs to be BoxedAnyObject");
                let view: std::cell::Ref<View> = entry.borrow();
                let tu_item: TuItem = glib::object::Object::new();
                tu_item.set_id(view.id.clone());
                tu_item.set_name(view.name.clone());
                let list_child = TuListItem::new(tu_item, "Views", false);
                list_item.set_child(Some(&list_child));
            }
        });
        imp.liblist.set_factory(Some(&factory));
        imp.liblist.set_model(Some(selection));
        let liblist = imp.liblist.get();
        liblist.connect_activate(
            glib::clone!(@weak self as obj => move |listview, position| {
                let model = listview.model().unwrap();
                let item = model.item(position).and_downcast::<glib::BoxedAnyObject>().unwrap();
                let view: std::cell::Ref<View> = item.borrow();
                let collection_type = match &view.collection_type {
                    Some(collection_type) => collection_type.clone(),
                    None => "".to_string(),
                };
                let item_page = ListPage::new(view.id.clone(),collection_type);
                item_page.set_tag(Some(&view.name));
                let window = obj.root().and_downcast::<Window>().unwrap();
                window.imp().homeview.push(&item_page);
                window.set_title(&view.name);
                window.change_pop_visibility();
                env::set_var("HOME_TITLE", &view.name)
            }),
        );
        libscrolled.set_child(Some(&liblist));

        let views =
            get_data_with_cache("0".to_string(), "views", async move { get_library().await })
                .await
                .unwrap();
        glib::spawn_future_local(glib::clone!(@weak self as obj =>async move {
                for view in &views {
                    let object = glib::BoxedAnyObject::new(view.clone());
                    store.append(&object);
                    gtk::glib::timeout_future(std::time::Duration::from_millis(30)).await;
                }
                obj.get_librarysscroll(&views).await;
        }));
    }

    pub async fn get_librarysscroll(&self, views: &[View]) {
        let libsrevealer = self.imp().libsrevealer.get();
        libsrevealer.set_reveal_child(true);
        let libsbox = self.imp().libsbox.get();
        for _ in 0..libsbox.observe_children().n_items() {
            libsbox.remove(&libsbox.last_child().unwrap());
        }
        for view in views.iter().cloned() {
            let libsbox = self.imp().libsbox.get();
            let scrolledwindow = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Automatic)
                .vscrollbar_policy(gtk::PolicyType::Never)
                .overlay_scrolling(true)
                .build();
            let scrolledwindow = fix(scrolledwindow);
            let scrollbox = gtk::Box::new(gtk::Orientation::Vertical, 15);
            let revealer = gtk::Revealer::builder()
                .reveal_child(false)
                .child(&scrollbox)
                .build();
            libsbox.append(&revealer);
            let view_name = view.name.replace('&', "&amp;");
            let label = gtk::Label::builder()
                .label(format!("<b>Latest {}</b>", view_name))
                .halign(gtk::Align::Start)
                .use_markup(true)
                .margin_top(15)
                .margin_start(10)
                .build();
            scrollbox.append(&label);
            scrollbox.append(&scrolledwindow);
            let latest = get_data_with_cache(view.id.clone(), "view", async move {
                get_latest(view.id.clone()).await
            })
            .await
            .unwrap();
            spawn(glib::clone!(@weak self as obj =>async move {
                    obj.set_librarysscroll(latest.clone());
                    let listview = obj.set_librarysscroll(latest);
                    scrolledwindow.set_child(Some(&listview));
                    if !revealer.reveals_child() {
                        revealer.set_reveal_child(true);
                    }
            }));
        }
        self.imp().spinner.set_visible(false);
    }

    pub fn set_librarysscroll(&self, latests: Vec<Latest>) -> gtk::ListView {
        let store = gtk::gio::ListStore::new::<glib::BoxedAnyObject>();

        let selection = gtk::SingleSelection::builder()
            .model(&store)
            .autoselect(false)
            .build();
        let factory = tu_list_item_factory("".to_string());
        let listview = gtk::ListView::new(Some(selection), Some(factory));
        listview.set_orientation(gtk::Orientation::Horizontal);
        listview.connect_activate(
            glib::clone!(@weak self as obj => move |listview, position| {
                    let window = obj.root().and_downcast::<Window>().unwrap();
                    let model = listview.model().unwrap();
                    let item = model.item(position).and_downcast::<glib::BoxedAnyObject>().unwrap();
                    let result: std::cell::Ref<Latest> = item.borrow();
                    tu_list_view_connect_activate(window, &result, None);
            }),
        );
        spawn(glib::clone!(@weak store => async move {
            for latest in latests {
                let object = glib::BoxedAnyObject::new(latest.clone());
                store.append(&object);
                gtk::glib::timeout_future(std::time::Duration::from_millis(30)).await;
            }
        }));
        listview
    }
}
