use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use glib::Object;
use gtk::{gio, glib};
use gtk::{template_callbacks, PositionType, ScrolledWindow};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::client::client::EMBY_CLIENT;
use crate::client::error::UserFacingError;
use crate::client::structs::*;
use crate::toast;

use crate::ui::provider::dropdown_factory::{factory, DropdownList, DropdownListBuilder};
use crate::ui::provider::tu_item::TuItem;
use crate::ui::provider::tu_object::TuObject;
use crate::utils::{get_image_with_cache, req_cache, spawn, spawn_tokio};
use chrono::{DateTime, Utc};

use super::fix::ScrolledWindowFixExt;
use super::hortu_scrolled::SHOW_BUTTON_ANIMATION_DURATION;
use super::song_widget::format_duration;
use super::tu_overview_item::run_time_ticks_to_label;
use super::utils::TuItemBuildExt;
use super::window::Window;

pub(crate) mod imp {
    use crate::ui::provider::tu_item::TuItem;
    use crate::ui::widgets::fix::ScrolledWindowFixExt;
    use crate::ui::widgets::horbu_scrolled::HorbuScrolled;
    use crate::ui::widgets::hortu_scrolled::HortuScrolled;
    use crate::ui::widgets::item_actionbox::ItemActionsBox;
    use crate::ui::widgets::item_carousel::ItemCarousel;
    use crate::ui::widgets::star_toggle::StarToggle;
    use crate::utils::spawn_g_timeout;
    use adw::subclass::prelude::*;
    use glib::subclass::InitializingObject;
    use gtk::prelude::*;
    use gtk::{glib, CompositeTemplate};
    use std::cell::{OnceCell, RefCell};

    // Object holding the state
    #[derive(CompositeTemplate, Default, glib::Properties)]
    #[template(resource = "/moe/tsukimi/item.ui")]
    #[properties(wrapper_type = super::ItemPage)]
    pub struct ItemPage {
        #[property(get, set, construct_only)]
        pub item: OnceCell<TuItem>,

        #[template_child]
        pub actorhortu: TemplateChild<HortuScrolled>,
        #[template_child]
        pub recommendhortu: TemplateChild<HortuScrolled>,
        #[template_child]
        pub includehortu: TemplateChild<HortuScrolled>,
        #[template_child]
        pub additionalhortu: TemplateChild<HortuScrolled>,

        #[template_child]
        pub studioshorbu: TemplateChild<HorbuScrolled>,
        #[template_child]
        pub tagshorbu: TemplateChild<HorbuScrolled>,
        #[template_child]
        pub genreshorbu: TemplateChild<HorbuScrolled>,
        #[template_child]
        pub linkshorbu: TemplateChild<HorbuScrolled>,

        #[template_child]
        pub itemlist: TemplateChild<gtk::ListView>,
        #[template_child]
        pub logobox: TemplateChild<gtk::Box>,
        #[template_child]
        pub seasonlist: TemplateChild<gtk::DropDown>,

        #[template_child]
        pub mediainfobox: TemplateChild<gtk::Box>,
        #[template_child]
        pub mediainforevealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub scrolled: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        pub episodesearchentry: TemplateChild<gtk::SearchEntry>,

        #[template_child]
        pub line1: TemplateChild<gtk::Label>,
        #[template_child]
        pub episode_line: TemplateChild<gtk::Label>,
        #[template_child]
        pub line2: TemplateChild<gtk::Label>,
        #[template_child]
        pub crating: TemplateChild<gtk::Label>,
        #[template_child]
        pub orating: TemplateChild<gtk::Label>,
        #[template_child]
        pub star: TemplateChild<gtk::Image>,

        #[template_child]
        pub playbutton: TemplateChild<gtk::Button>,
        #[template_child]
        pub namedropdown: TemplateChild<gtk::DropDown>,
        #[template_child]
        pub subdropdown: TemplateChild<gtk::DropDown>,
        #[template_child]
        pub carousel: TemplateChild<ItemCarousel>,
        #[template_child]
        pub actionbox: TemplateChild<ItemActionsBox>,
        #[template_child]
        pub tagline: TemplateChild<gtk::Label>,
        #[template_child]
        pub toolbar: TemplateChild<gtk::Box>,

        #[template_child]
        pub buttoncontent: TemplateChild<adw::ButtonContent>,

        pub selection: gtk::SingleSelection,
        pub seasonselection: gtk::SingleSelection,
        pub playbuttonhandlerid: RefCell<Option<glib::SignalHandlerId>>,

        #[property(get, set, construct_only)]
        pub name: RefCell<Option<String>>,
        pub selected: RefCell<Option<String>>,

        pub videoselection: gtk::SingleSelection,
        pub subselection: gtk::SingleSelection,

        #[template_child]
        pub main_carousel: TemplateChild<adw::Carousel>,

        #[template_child]
        pub left_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub right_button: TemplateChild<gtk::Button>,

        pub show_button_animation: OnceCell<adw::TimedAnimation>,
        pub hide_button_animation: OnceCell<adw::TimedAnimation>,

        #[property(get, set, nullable)]
        pub current_item: RefCell<Option<TuItem>>,
        #[property(get, set, nullable)]
        pub play_session_id: RefCell<Option<String>>,
    }

    // The central trait for subclassing a GObject
    #[glib::object_subclass]
    impl ObjectSubclass for ItemPage {
        // `NAME` needs to match `class` attribute of template
        const NAME: &'static str = "ItemPage";
        type Type = super::ItemPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            ItemCarousel::ensure_type();
            StarToggle::ensure_type();
            HortuScrolled::ensure_type();
            HorbuScrolled::ensure_type();
            klass.bind_template();
            klass.bind_template_instance_callbacks();
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    // Trait shared by all GObjects
    #[glib::derived_properties]
    impl ObjectImpl for ItemPage {
        fn constructed(&self) {
            self.parent_constructed();
            self.scrolled.fix();
            let obj = self.obj();
            spawn_g_timeout(glib::clone!(
                #[weak]
                obj,
                async move {
                    obj.setup().await;
                }
            ));
        }
    }

    // Trait shared by all widgets
    impl WidgetImpl for ItemPage {}

    // Trait shared by all windows
    impl WindowImpl for ItemPage {}

    // Trait shared by all application windows
    impl ApplicationWindowImpl for ItemPage {}

    impl adw::subclass::navigation_page::NavigationPageImpl for ItemPage {}
}

glib::wrapper! {
    pub struct ItemPage(ObjectSubclass<imp::ItemPage>)
        @extends gtk::ApplicationWindow, gtk::Window, gtk::Widget ,adw::NavigationPage,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

#[template_callbacks]
impl ItemPage {
    pub fn new(item: &TuItem) -> Self {
        Object::builder().property("item", item).build()
    }

    pub async fn setup(&self) {
        let item = self.item();
        let type_ = item.item_type();
        let imp = self.imp();

        if let Some(series_name) = item.series_name() {
            imp.line1.set_text(&series_name);
        } else {
            imp.line1.set_text(&item.name());
        }

        if type_ == "Series" || type_ == "Episode" {
            self.imp().toolbar.set_visible(true);
            let series_id = item.series_id().unwrap_or(item.id());

            spawn(glib::clone!(
                #[weak(rename_to = obj)]
                self,
                #[strong]
                series_id,
                async move {
                    let Some(intro) = obj.set_shows_next_up(&series_id).await else {
                        return;
                    };
                    obj.set_intro::<false>(&intro).await;
                }
            ));

            self.imp().actionbox.set_id(Some(series_id.clone()));
            self.setup_item(&series_id).await;
            self.setup_seasons(&series_id).await;
        } else {
            let id = item.id();

            spawn(glib::clone!(
                #[weak(rename_to = obj)]
                self,
                async move {
                    obj.set_intro::<true>(&item).await;
                }
            ));

            self.imp().actionbox.set_id(Some(id.clone()));
            self.setup_item(&id).await;
        }
    }

    async fn setup_item(&self, id: &str) {
        let id = id.to_string();
        let id_clone = id.clone();

        spawn(glib::clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                obj.set_logo(&id_clone);
            }
        ));

        self.setup_background(&id).await;
        self.set_overview(&id).await;
        self.set_lists(&id).await;
    }

    async fn set_intro<const IS_VIDEO: bool>(&self, intro: &TuItem) {
        let intro_id = intro.id();
        let play_button = self.imp().playbutton.get();

        self.set_now_item::<IS_VIDEO>(&intro);

        play_button.set_sensitive(false);

        let playback =
            match spawn_tokio(async move { EMBY_CLIENT.get_playbackinfo(&intro_id).await }).await {
                Ok(playback) => playback,
                Err(e) => {
                    toast!(self, e.to_user_facing());
                    return;
                }
            };

        self.set_dropdown(&playback);

        self.set_current_item(Some(intro));

        play_button.set_sensitive(true);
    }

    async fn set_shows_next_up(&self, id: &str) -> Option<TuItem> {
        let id = id.to_string();
        let next_up =
            match spawn_tokio(async move { EMBY_CLIENT.get_shows_next_up(&id).await }).await {
                Ok(next_up) => next_up,
                Err(e) => {
                    toast!(self, e.to_user_facing());
                    return None;
                }
            };

        let next_up_item = next_up.items.first()?;

        let imp = self.imp();
        imp.episode_line.set_visible(true);

        self.set_now_item::<false>(&TuItem::from_simple(next_up_item, None));

        Some(TuItem::from_simple(next_up_item, None))
    }

    fn set_now_item<const IS_VIDEO: bool>(&self, item: &TuItem) {
        let imp = self.imp();

        if IS_VIDEO {
            imp.episode_line.set_text(&item.name());
        } else {
            imp.episode_line.set_text(&format!(
                "S{}E{}: {}",
                item.parent_index_number(),
                item.index_number(),
                item.name()
            ));
        }

        let sec = item.playback_position_ticks() / 10000000;
        if sec > 10 {
            imp.buttoncontent.set_label(&format!(
                "{} {}",
                gettext("Resume"),
                format_duration(sec as i64)
            ));
        } else {
            imp.buttoncontent.set_label(&gettext("Play"));
        }
    }

    pub fn set_dropdown(&self, playbackinfo: &Media) {
        let playbackinfo = playbackinfo.clone();
        let imp = self.imp();
        let namedropdown = imp.namedropdown.get();
        let subdropdown = imp.subdropdown.get();
        namedropdown.set_factory(Some(&factory::<true>()));
        namedropdown.set_list_factory(Some(&factory::<false>()));
        subdropdown.set_factory(Some(&factory::<true>()));
        subdropdown.set_list_factory(Some(&factory::<false>()));

        let vstore = gtk::gio::ListStore::new::<glib::BoxedAnyObject>();
        imp.videoselection.set_model(Some(&vstore));

        let sstore = gtk::gio::ListStore::new::<glib::BoxedAnyObject>();
        imp.subselection.set_model(Some(&sstore));

        namedropdown.set_model(Some(&imp.videoselection));
        subdropdown.set_model(Some(&imp.subselection));

        let media_sources = playbackinfo.media_sources.clone();

        namedropdown.connect_selected_item_notify(move |dropdown| {
            let Some(entry) = dropdown
                .selected_item()
                .and_downcast::<glib::BoxedAnyObject>()
            else {
                return;
            };

            let dl: std::cell::Ref<DropdownList> = entry.borrow();
            let selected = &dl.id;
            for _i in 0..sstore.n_items() {
                sstore.remove(0);
            }
            for media in &media_sources {
                if &Some(media.id.clone()) == selected {
                    for stream in &media.media_streams {
                        if stream.stream_type == "Subtitle" {
                            let Ok(dl) = DropdownListBuilder::default()
                                .line1(stream.display_title.clone())
                                .line2(stream.title.clone())
                                .index(Some(stream.index.clone()))
                                .direct_url(stream.delivery_url.clone())
                                .build()
                            else {
                                continue;
                            };

                            let object = glib::BoxedAnyObject::new(dl);
                            sstore.append(&object);
                        }
                    }
                    subdropdown.set_selected(0);
                    break;
                }
            }
        });

        for media in &playbackinfo.media_sources {
            let Ok(dl) = DropdownListBuilder::default()
                .line1(Some(media.name.clone()))
                .line2(Some(media.container.clone()))
                .direct_url(media.direct_stream_url.clone())
                .id(Some(media.id.clone()))
                .build()
            else {
                continue;
            };

            let object = glib::BoxedAnyObject::new(dl);
            vstore.append(&object);
        }

        namedropdown.set_selected(0);
    }

    pub async fn setup_background(&self, id: &str) {
        let imp = self.imp();

        let backdrop = imp.carousel.imp().backdrop.get();
        let path = get_image_with_cache(&id, "Backdrop", Some(0))
            .await
            .unwrap();
        let file = gtk::gio::File::for_path(&path);
        let pathbuf = PathBuf::from(&path);
        if pathbuf.exists() {
            backdrop.set_file(Some(&file));
            self.imp()
                .carousel
                .imp()
                .backrevealer
                .set_reveal_child(true);
            spawn(glib::clone!(
                #[weak(rename_to = obj)]
                self,
                async move {
                    let Some(window) = obj.root().and_downcast::<super::window::Window>() else {
                        return;
                    };
                    window.set_rootpic(file);
                }
            ));
        }
    }

    pub async fn add_backdrops(&self, image_tags: Vec<String>) {
        let imp = self.imp();
        let id = self.item().id();
        let tags = image_tags.len();
        let carousel = imp.carousel.imp().carousel.get();
        for tag_num in 1..tags {
            let path = get_image_with_cache(&id, "Backdrop", Some(tag_num as u8))
                .await
                .unwrap();
            let file = gtk::gio::File::for_path(&path);
            let picture = gtk::Picture::builder()
                .halign(gtk::Align::Fill)
                .valign(gtk::Align::Fill)
                .content_fit(gtk::ContentFit::Cover)
                .file(&file)
                .build();
            carousel.append(&picture);
            carousel.set_allow_scroll_wheel(true);
        }

        if carousel.n_pages() == 1 {
            return;
        }

        glib::timeout_add_seconds_local(10, move || {
            let current_page = carousel.position();
            let n_pages = carousel.n_pages();
            let new_page_position = (current_page + 1. + n_pages as f64) % n_pages as f64;
            carousel.scroll_to(&carousel.nth_page(new_page_position as u32), true);

            glib::ControlFlow::Continue
        });
    }

    pub async fn setup_seasons(&self, id: &str) {
        let imp = self.imp();
        let id = id.to_string();

        let store = gtk::gio::ListStore::new::<TuObject>();
        imp.selection.set_autoselect(false);
        imp.selection.set_model(Some(&store));

        let seasonstore = gtk::StringList::new(&[]);
        imp.seasonselection.set_model(Some(&seasonstore));
        let seasonlist = imp.seasonlist.get();
        seasonlist.set_model(Some(&imp.seasonselection));

        let factory = gtk::SignalListItemFactory::new();
        factory.tu_overview_item();
        imp.itemlist.set_factory(Some(&factory));
        imp.itemlist.set_model(Some(&imp.selection));

        let series_info =
            match spawn_tokio(async move { EMBY_CLIENT.get_series_info(&id).await }).await {
                Ok(item) => item.items,
                Err(e) => {
                    toast!(self, e.to_user_facing());
                    Vec::new()
                }
            };

        spawn(glib::clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                let mut season_set: HashSet<u32> = HashSet::new();
                let mut season_map: HashMap<String, u32> = HashMap::new();
                let min_season = series_info
                    .iter()
                    .map(|info| {
                        if info.parent_index_number.unwrap_or(0) == 0 {
                            100
                        } else {
                            info.parent_index_number.unwrap_or(0)
                        }
                    })
                    .min()
                    .unwrap_or(1);
                let mut pos = 0;
                let mut set = true;
                for info in &series_info {
                    if !season_set.contains(&info.parent_index_number.unwrap_or(0)) {
                        let seasonstring =
                            format!("Season {}", info.parent_index_number.unwrap_or(0));
                        seasonstore.append(&seasonstring);
                        season_set.insert(info.parent_index_number.unwrap_or(0));
                        season_map
                            .insert(seasonstring.clone(), info.parent_index_number.unwrap_or(0));
                        if set {
                            if info.parent_index_number.unwrap_or(0) == min_season {
                                set = false;
                            } else {
                                pos += 1;
                            }
                        }
                    }
                    if info.parent_index_number.unwrap_or(0) == min_season {
                        let tu_item = TuItem::from_simple(&info, None);
                        let object = TuObject::new(&tu_item);
                        store.append(&object);
                    }
                }
                obj.imp().seasonlist.set_selected(pos);
                let seasonlist = obj.imp().seasonlist.get();
                let seriesinfo_seasonlist = series_info.clone();
                let seriesinfo_seasonmap = season_map.clone();
                seasonlist.connect_selected_item_notify(glib::clone!(
                    #[weak]
                    store,
                    move |dropdown| {
                        let selected = dropdown.selected_item();
                        let selected = selected.and_downcast_ref::<gtk::StringObject>().unwrap();
                        let selected = selected.string().to_string();
                        store.remove_all();
                        let season_number = seriesinfo_seasonmap[&selected];
                        for info in &seriesinfo_seasonlist {
                            if info.parent_index_number.unwrap_or(0) == season_number {
                                let tu_item = TuItem::from_simple(&info, None);
                                let object = TuObject::new(&tu_item);
                                store.append(&object);
                            }
                        }
                    }
                ));
                let episodesearchentry = obj.imp().episodesearchentry.get();
                episodesearchentry.connect_search_changed(glib::clone!(
                    #[weak]
                    store,
                    move |entry| {
                        let text = entry.text();
                        store.remove_all();
                        for info in &series_info {
                            if (info.name.to_lowercase().contains(&text.to_lowercase())
                                || info
                                    .index_number
                                    .unwrap_or(0)
                                    .to_string()
                                    .contains(&text.to_lowercase()))
                                && info.parent_index_number.unwrap_or(0)
                                    == season_map[&seasonlist
                                        .selected_item()
                                        .and_downcast_ref::<gtk::StringObject>()
                                        .unwrap()
                                        .string()
                                        .to_string()]
                            {
                                let tu_item = TuItem::from_simple(&info, None);
                                let object = TuObject::new(&tu_item);
                                store.append(&object);
                            }
                        }
                    }
                ));
            }
        ));

        imp.itemlist.connect_activate(glib::clone!(
            #[weak(rename_to = obj)]
            self,
            move |listview, position| {
                let model = listview.model().unwrap();
                let item = model.item(position).and_downcast::<TuObject>().unwrap();
                spawn(glib::clone!(
                    #[weak]
                    obj,
                    async move {
                        obj.set_intro::<false>(&item.item()).await;
                    }
                ));
            }
        ));
    }

    pub fn set_logo(&self, id: &str) {
        let logo = super::logo::set_logo(id.to_string(), "Logo", None);
        self.imp().logobox.append(&logo);
    }

    pub async fn set_overview(&self, id: &str) {
        let id = id.to_string();

        let item = match req_cache(&format!("item_{}", &id), async move {
            EMBY_CLIENT.get_item_info(&id).await
        })
        .await
        {
            Ok(item) => item,
            Err(e) => {
                toast!(self, e.to_user_facing());
                Item::default()
            }
        };

        spawn(glib::clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                {
                    let mut str = String::new();
                    if let Some(communityrating) = item.community_rating {
                        let formatted_rating = format!("{:.1}", communityrating);
                        let crating = obj.imp().crating.get();
                        crating.set_text(&formatted_rating);
                        crating.set_visible(true);
                        obj.imp().star.get().set_visible(true);
                    }
                    if let Some(rating) = item.official_rating {
                        let orating = obj.imp().orating.get();
                        orating.set_text(&rating);
                        orating.set_visible(true);
                    }
                    if let Some(year) = item.production_year {
                        str.push_str(&year.to_string());
                        str.push_str("  ");
                    }
                    if let Some(runtime) = item.run_time_ticks {
                        let time_string = run_time_ticks_to_label(runtime);
                        str.push_str(&time_string);
                        str.push_str("  ");
                    }
                    if let Some(genres) = &item.genres {
                        for genre in genres {
                            str.push_str(&genre.name);
                            str.push(',');
                        }
                        str.pop();
                    }
                    obj.imp().line2.get().set_text(&str);

                    if let Some(taglines) = item.taglines {
                        if let Some(tagline) = taglines.first() {
                            obj.imp().tagline.set_text(tagline);
                            obj.imp().tagline.set_visible(true);
                        }
                    }
                }
                if let Some(links) = item.external_urls {
                    obj.set_flowlinks(links);
                }
                if let Some(actor) = item.people {
                    obj.setactorscrolled(actor).await;
                }
                if let Some(studios) = item.studios {
                    obj.set_flowbuttons(studios, "Studios");
                }
                if let Some(tags) = item.tags {
                    obj.set_flowbuttons(tags, "Tags");
                }
                if let Some(genres) = item.genres {
                    obj.set_flowbuttons(genres, "Genres");
                }
                if let Some(image_tags) = item.backdrop_image_tags {
                    obj.add_backdrops(image_tags).await;
                }
                if let Some(ref user_data) = item.user_data {
                    let imp = obj.imp();
                    if let Some(is_favourite) = user_data.is_favorite {
                        imp.actionbox.set_btn_active(is_favourite);
                    }
                    imp.actionbox.set_played(user_data.played);
                    imp.actionbox.bind_edit();
                }

                if let Some(media_sources) = item.media_sources {
                    obj.createmediabox(media_sources, item.date_created).await;
                }
            }
        ));
    }

    pub async fn createmediabox(
        &self,
        media_sources: Vec<MediaSource>,
        date_created: Option<DateTime<Utc>>,
    ) {
        let imp = self.imp();
        let mediainfobox = imp.mediainfobox.get();
        let mediainforevealer = imp.mediainforevealer.get();

        while mediainfobox.last_child().is_some() {
            if let Some(child) = mediainfobox.last_child() {
                mediainfobox.remove(&child)
            }
        }
        for mediasource in media_sources {
            let singlebox = gtk::Box::new(gtk::Orientation::Vertical, 5);
            let info = format!(
                "{}\n{} {} {}\n{}",
                mediasource.path.unwrap_or_default(),
                mediasource.container.to_uppercase(),
                bytefmt::format(mediasource.size),
                dt(date_created),
                mediasource.name
            );
            let label = gtk::Label::builder()
                .label(&info)
                .halign(gtk::Align::Start)
                .margin_start(15)
                .valign(gtk::Align::Start)
                .margin_top(5)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            label.add_css_class("caption-heading");
            singlebox.append(&label);

            let mediascrolled = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Automatic)
                .vscrollbar_policy(gtk::PolicyType::Never)
                .margin_start(15)
                .margin_end(15)
                .overlay_scrolling(true)
                .build();

            let mediascrolled = mediascrolled.fix();

            let mediabox = gtk::Box::new(gtk::Orientation::Horizontal, 5);
            for mediapart in mediasource.media_streams {
                if mediapart.stream_type == "Attachment" {
                    continue;
                }
                let mediapartbox = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .spacing(0)
                    .width_request(300)
                    .build();
                let mut str: String = Default::default();
                let icon = gtk::Image::builder().margin_end(5).build();
                if mediapart.stream_type == "Video" {
                    icon.set_icon_name(Some("video-x-generic-symbolic"))
                } else if mediapart.stream_type == "Audio" {
                    icon.set_icon_name(Some("audio-x-generic-symbolic"))
                } else if mediapart.stream_type == "Subtitle" {
                    icon.set_icon_name(Some("media-view-subtitles-symbolic"))
                } else {
                    icon.set_icon_name(Some("text-x-generic-symbolic"))
                }
                let typebox = gtk::Box::builder()
                    .orientation(gtk::Orientation::Horizontal)
                    .spacing(5)
                    .build();
                typebox.append(&icon);
                typebox.append(&gtk::Label::new(Some(&mediapart.stream_type)));
                if let Some(codec) = mediapart.codec {
                    str.push_str(format!("Codec: {}", codec).as_str());
                }
                if let Some(language) = mediapart.display_language {
                    str.push_str(format!("\nLanguage: {}", language).as_str());
                }
                if let Some(title) = mediapart.title {
                    str.push_str(format!("\nTitle: {}", title).as_str());
                }
                if let Some(bitrate) = mediapart.bit_rate {
                    str.push_str(format!("\nBitrate: {}it/s", bytefmt::format(bitrate)).as_str());
                }
                if let Some(bitdepth) = mediapart.bit_depth {
                    str.push_str(format!("\nBitDepth: {} bit", bitdepth).as_str());
                }
                if let Some(samplerate) = mediapart.sample_rate {
                    str.push_str(format!("\nSampleRate: {} Hz", samplerate).as_str());
                }
                if let Some(height) = mediapart.height {
                    str.push_str(format!("\nHeight: {}", height).as_str());
                }
                if let Some(width) = mediapart.width {
                    str.push_str(format!("\nWidth: {}", width).as_str());
                }
                if let Some(colorspace) = mediapart.color_space {
                    str.push_str(format!("\nColorSpace: {}", colorspace).as_str());
                }
                if let Some(displaytitle) = mediapart.display_title {
                    str.push_str(format!("\nDisplayTitle: {}", displaytitle).as_str());
                }
                if let Some(channel) = mediapart.channels {
                    str.push_str(format!("\nChannel: {}", channel).as_str());
                }
                if let Some(channellayout) = mediapart.channel_layout {
                    str.push_str(format!("\nChannelLayout: {}", channellayout).as_str());
                }
                if let Some(averageframerate) = mediapart.average_frame_rate {
                    str.push_str(format!("\nAverageFrameRate: {}", averageframerate).as_str());
                }
                if let Some(pixelformat) = mediapart.pixel_format {
                    str.push_str(format!("\nPixelFormat: {}", pixelformat).as_str());
                }
                let inscription = gtk::Inscription::builder()
                    .text(&str)
                    .min_lines(14)
                    .hexpand(true)
                    .margin_start(15)
                    .margin_end(15)
                    .yalign(0.0)
                    .build();
                mediapartbox.append(&typebox);
                mediapartbox.append(&inscription);
                mediapartbox.add_css_class("card");
                mediapartbox.add_css_class("sbackground");
                mediabox.append(&mediapartbox);
            }

            mediascrolled.set_child(Some(&mediabox));
            singlebox.append(mediascrolled);
            mediainfobox.append(&singlebox);
        }
        mediainforevealer.set_reveal_child(true);
    }

    pub async fn setactorscrolled(&self, actors: Vec<SimpleListItem>) {
        let hortu = self.imp().actorhortu.get();

        hortu.set_title("Actors");

        hortu.set_items(&actors);
    }

    pub async fn set_lists(&self, id: &str) {
        self.sets("Recommend", id).await;
        self.sets("Included In", id).await;
        self.sets("Additional Parts", id).await;
    }

    pub async fn sets(&self, types: &str, id: &str) {
        let hortu = match types {
            "Recommend" => self.imp().recommendhortu.get(),
            "Included In" => self.imp().includehortu.get(),
            "Additional Parts" => self.imp().additionalhortu.get(),
            _ => return,
        };

        hortu.set_title(types);

        let id = id.to_string();
        let types = types.to_string();

        let results = match req_cache(&format!("item_{types}_{id}"), async move {
            match types.as_str() {
                "Recommend" => EMBY_CLIENT.get_similar(&id).await,
                "Included In" => EMBY_CLIENT.get_included(&id).await,
                "Additional Parts" => EMBY_CLIENT.get_additional(&id).await,
                _ => Ok(List::default()),
            }
        })
        .await
        {
            Ok(history) => history,
            Err(e) => {
                toast!(self, e.to_user_facing());
                List::default()
            }
        };

        hortu.set_items(&results.items);
    }

    pub fn set_flowbuttons(&self, infos: Vec<SGTitem>, type_: &str) {
        let imp = self.imp();
        let horbu = match type_ {
            "Genres" => imp.genreshorbu.get(),
            "Studios" => imp.studioshorbu.get(),
            "Tags" => imp.tagshorbu.get(),
            _ => return,
        };

        horbu.set_title(type_);

        horbu.set_list_type(Some(type_.to_string()));

        horbu.set_items(&infos);
    }

    pub fn set_flowlinks(&self, links: Vec<Urls>) {
        let imp = self.imp();

        let horbu = imp.linkshorbu.get();

        horbu.set_title("Links");

        horbu.set_links(&links);
    }

    pub fn get_window(&self) -> Window {
        self.root().unwrap().downcast::<Window>().unwrap()
    }

    #[template_callback]
    fn edge_overshot_cb(&self, pos: PositionType, _window: &ScrolledWindow) {
        if pos != gtk::PositionType::Top {
            return;
        }

        let carousel = self.imp().main_carousel.get();
        carousel.scroll_to(&carousel.nth_page(0), true);
    }

    #[template_callback]
    async fn play_cb(&self) {
        let video_dropdown = self.imp().namedropdown.get();
        let sub_dropdown = self.imp().subdropdown.get();

        let Some(video_object) = video_dropdown.selected_item().and_downcast::<glib::BoxedAnyObject>() else {
            return;
        };

        let video_dl: std::cell::Ref<DropdownList> = video_object.borrow();

        let Some(ref video_url) = video_dl.direct_url else {
            toast!(self, "No video source found");
            return;
        };

        let Some(ref media_source_id) = video_dl.id else {
            return;
        };

        let Some(item) = self.current_item() else {
            return;
        };

        let back = Back {
            id: item.id(),
            playsessionid: self.play_session_id(),
            mediasourceid: media_source_id.to_string(),
            tick: 0,
        };

        let sub_url = if let Some(sub_object) = sub_dropdown.selected_item().and_downcast::<glib::BoxedAnyObject>() {
            let sub_dl: std::cell::Ref<DropdownList> = sub_object.borrow();
            sub_dl.direct_url.clone()
        } else {
            None
        };

        

        let percentage = item.played_percentage();

        self.get_window().play_media(video_url.to_string(), sub_url, item.name(), Some(back), None, percentage);
    }

    fn set_control_opacity(&self, opacity: f64) {
        let imp = self.imp();
        imp.left_button.set_opacity(opacity);
        imp.right_button.set_opacity(opacity);
    }

    fn are_controls_visible(&self) -> bool {
        if self.hide_controls_animation().state() == adw::AnimationState::Playing {
            return false;
        }

        self.imp().left_button.opacity() >= 0.68
            || self.show_controls_animation().state() == adw::AnimationState::Playing
    }

    fn show_controls_animation(&self) -> &adw::TimedAnimation {
        self.imp().show_button_animation.get_or_init(|| {
            let target = adw::CallbackAnimationTarget::new(glib::clone!(
                #[weak(rename_to = obj)]
                self,
                move |opacity| obj.set_control_opacity(opacity)
            ));

            adw::TimedAnimation::builder()
                .duration(SHOW_BUTTON_ANIMATION_DURATION)
                .widget(&self.imp().scrolled.get())
                .target(&target)
                .value_to(0.7)
                .build()
        })
    }

    fn hide_controls_animation(&self) -> &adw::TimedAnimation {
        self.imp().hide_button_animation.get_or_init(|| {
            let target = adw::CallbackAnimationTarget::new(glib::clone!(
                #[weak(rename_to = obj)]
                self,
                move |opacity| obj.set_control_opacity(opacity)
            ));

            adw::TimedAnimation::builder()
                .duration(SHOW_BUTTON_ANIMATION_DURATION)
                .widget(&self.imp().scrolled.get())
                .target(&target)
                .value_to(0.)
                .build()
        })
    }

    #[template_callback]
    fn on_rightbutton_clicked(&self) {
        self.anime::<true>();
    }

    fn controls_opacity(&self) -> f64 {
        self.imp().left_button.opacity()
    }

    #[template_callback]
    fn on_enter_focus(&self) {
        if !self.are_controls_visible() {
            self.hide_controls_animation().pause();
            self.show_controls_animation()
                .set_value_from(self.controls_opacity());
            self.show_controls_animation().play();
        }
    }

    #[template_callback]
    fn on_leave_focus(&self) {
        if self.are_controls_visible() {
            self.show_controls_animation().pause();
            self.hide_controls_animation()
                .set_value_from(self.controls_opacity());
            self.hide_controls_animation().play();
        }
    }

    #[template_callback]
    fn on_leftbutton_clicked(&self) {
        self.anime::<false>();
    }

    fn anime<const R: bool>(&self) {
        let scrolled = self.imp().scrolled.get();
        let adj = scrolled.hadjustment();

        let Some(clock) = scrolled.frame_clock() else {
            return;
        };

        let start = adj.value();
        let end = if R { start + 800.0 } else { start - 800.0 };

        let start_time = clock.frame_time();
        let end_time = start_time + 1000 * 400;

        scrolled.add_tick_callback(move |_view, clock| {
            let now = clock.frame_time();
            if now < end_time && adj.value() != end {
                let mut t = (now - start_time) as f64 / (end_time - start_time) as f64;
                t = Self::ease_in_out_cubic(t);
                adj.set_value(start + t * (end - start));
                glib::ControlFlow::Continue
            } else {
                adj.set_value(end);
                glib::ControlFlow::Break
            }
        });
    }

    fn ease_in_out_cubic(t: f64) -> f64 {
        if t < 0.5 {
            4.0 * t * t * t
        } else {
            let t = 2.0 * t - 2.0;
            0.5 * t * t * t + 1.0
        }
    }
}

pub fn dt(date: Option<chrono::DateTime<Utc>>) -> String {
    let Some(date) = date else {
        return "".to_string();
    };
    date.format("%Y-%m-%d %H:%M:%S").to_string()
}
