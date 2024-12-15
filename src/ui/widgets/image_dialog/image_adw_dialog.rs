use adw::subclass::prelude::*;
use gtk::{
    glib,
    prelude::*,
    template_callbacks,
};

use crate::{
    client::{
        emby_client::EMBY_CLIENT,
        error::UserFacingError,
    },
    toast,
    utils::spawn_tokio,
};

mod imp {
    use std::cell::OnceCell;

    use adw::prelude::*;
    use glib::subclass::InitializingObject;
    use gtk::{
        glib,
        CompositeTemplate,
    };

    use super::*;
    use crate::{
        client::structs::ImageItem,
        ui::{
            provider::IS_ADMIN,
            widgets::image_dialog::ImageInfoCard,
        },
        utils::spawn,
    };

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/moe/tsuna/tsukimi/ui/images_dialog.ui")]
    #[properties(wrapper_type = super::ImagesDialog)]
    pub struct ImagesDialog {
        #[property(get, set, construct_only)]
        pub id: OnceCell<String>,

        #[template_child]
        pub hint: TemplateChild<adw::ActionRow>,

        #[template_child]
        pub page: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub view: TemplateChild<adw::NavigationView>,

        #[template_child]
        pub primary: TemplateChild<ImageInfoCard>,
        #[template_child]
        pub logo: TemplateChild<ImageInfoCard>,
        #[template_child]
        pub thumb: TemplateChild<ImageInfoCard>,
        #[template_child]
        pub banner: TemplateChild<ImageInfoCard>,
        #[template_child]
        pub disc: TemplateChild<ImageInfoCard>,
        #[template_child]
        pub art: TemplateChild<ImageInfoCard>,

        #[template_child]
        pub flowbox: TemplateChild<gtk::FlowBox>,

        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,

        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,

        #[template_child]
        pub size_group: TemplateChild<gtk::SizeGroup>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ImagesDialog {
        const NAME: &'static str = "ImagesDialog";
        type Type = super::ImagesDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            ImageInfoCard::ensure_type();
            klass.bind_template();
            klass.bind_template_instance_callbacks();
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ImagesDialog {
        fn constructed(&self) {
            self.parent_constructed();

            let id = self.obj().id();

            self.primary.set_imgid(id.clone());
            self.logo.set_imgid(id.clone());
            self.thumb.set_imgid(id.clone());
            self.banner.set_imgid(id.clone());
            self.disc.set_imgid(id.clone());
            self.art.set_imgid(id.clone());

            self.size_group.add_widget(&self.primary.imp().stack.get());
            self.size_group.add_widget(&self.logo.imp().stack.get());
            self.size_group.add_widget(&self.thumb.imp().stack.get());
            self.size_group.add_widget(&self.banner.imp().stack.get());
            self.size_group.add_widget(&self.disc.imp().stack.get());
            self.size_group.add_widget(&self.art.imp().stack.get());

            self.init();
        }
    }

    impl WidgetImpl for ImagesDialog {}
    impl AdwDialogImpl for ImagesDialog {}

    impl ImagesDialog {
        fn init(&self) {
            if IS_ADMIN.load(std::sync::atomic::Ordering::Relaxed) {
                self.page.set_title("View Images");
                self.hint
                    .set_subtitle("This page is READ-ONLY, because it is not finished yet.");
            }

            let obj = self.obj();
            spawn(glib::clone!(
                #[weak]
                obj,
                async move {
                    obj.set_image_items().await;
                }
            ));
        }

        pub fn set_card(&self, card: &ImageInfoCard, item: &ImageItem) {
            card.set_loading_visible();
            card.set_size(&item.width, &item.height, &item.size);
            card.set_picture(&item.image_type, &self.obj().id(), &None);
        }

        pub fn add_backdrop(&self, item: &ImageItem) {
            let card = ImageInfoCard::new("Backdrop", &self.obj().id());
            card.set_loading_visible();
            card.set_size(&item.width, &item.height, &item.size);
            card.set_picture(&item.image_type, &self.obj().id(), &item.image_index);
            self.size_group.add_widget(&card.imp().stack.get());
            self.flowbox.append(&card);
        }

        pub fn set_item(&self, item: &ImageItem) {
            match item.image_type.as_str() {
                "Primary" => {
                    self.set_card(&self.primary, item);
                }
                "Logo" => {
                    self.set_card(&self.logo, item);
                }
                "Thumb" => {
                    self.set_card(&self.thumb, item);
                }
                "Banner" => {
                    self.set_card(&self.banner, item);
                }
                "Disc" => {
                    self.set_card(&self.disc, item);
                }
                "Art" => {
                    self.set_card(&self.art, item);
                }
                "Backdrop" => {
                    self.add_backdrop(item);
                }
                _ => {}
            }
        }
    }
}

glib::wrapper! {
    /// Preference Window to display and update room details.
    pub struct ImagesDialog(ObjectSubclass<imp::ImagesDialog>)
        @extends gtk::Widget, adw::Dialog, adw::PreferencesDialog, @implements gtk::Accessible, gtk::Root;
}

#[template_callbacks]
impl ImagesDialog {
    const LOADING_STACK_PAGE: &'static str = "loading";
    const VIEW_STACK_PAGE: &'static str = "view";

    pub fn new(id: &str) -> Self {
        glib::Object::builder().property("id", id).build()
    }

    pub fn loading_page(&self) {
        self.imp()
            .stack
            .set_visible_child_name(Self::LOADING_STACK_PAGE);
    }

    pub fn view_page(&self) {
        self.imp()
            .stack
            .set_visible_child_name(Self::VIEW_STACK_PAGE);
    }

    pub fn add_toast(&self, toast: adw::Toast) {
        self.imp().toast_overlay.add_toast(toast);
    }

    pub async fn set_image_items(&self) {
        let id = self.id();
        match spawn_tokio(async move { EMBY_CLIENT.get_image_items(&id).await }).await {
            Ok(items) => {
                for item in items {
                    self.imp().set_item(&item);
                }
            }
            Err(e) => {
                toast!(self, e.to_user_facing());
            }
        }

        self.view_page();
    }

    pub fn pop_page(&self) {
        self.imp().view.pop();
    }
}
