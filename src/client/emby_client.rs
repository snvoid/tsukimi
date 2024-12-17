use std::{
    hash::Hasher,
    sync::{
        Arc,
        Mutex,
    },
};

use anyhow::{
    anyhow,
    Result,
};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::{
    header::HeaderValue,
    Client,
    Method,
    RequestBuilder,
    Response,
};
use serde::{
    de::DeserializeOwned,
    Deserialize,
    Serialize,
};
use serde_json::{
    json,
    Value,
};
use tracing::{
    debug,
    warn,
};
use url::Url;
use uuid::Uuid;

use super::{
    error::UserFacingError,
    structs::{
        ActivityLogs,
        AuthenticateResponse,
        Back,
        ExternalIdInfo,
        ImageItem,
        ImageSearchResult,
        List,
        LiveMedia,
        LoginResponse,
        Media,
        PublicServerInfo,
        RemoteSearchInfo,
        RemoteSearchResult,
        ScheduledTask,
        ServerInfo,
        SimpleListItem,
    },
    Account,
    ReqClient,
};
use crate::{
    config::VERSION,
    ui::{
        models::{
            emby_cache_path,
            SETTINGS,
        },
        widgets::single_grid::imp::ListType,
    },
    utils::spawn_tokio_without_await,
};

pub static EMBY_CLIENT: Lazy<EmbyClient> = Lazy::new(EmbyClient::default);
pub static DEVICE_ID: Lazy<String> = Lazy::new(|| {
    let uuid = SETTINGS.device_uuid();
    if uuid.is_empty() {
        let uuid = Uuid::new_v4().to_string();
        let _ = SETTINGS.set_device_uuid(&uuid);
        uuid
    } else {
        uuid
    }
});

const PROFILE: &str = include_str!("stream_profile.json");
const CLIENT_ID: &str = "Tsukimi";

static DEVICE_NAME: Lazy<String> = Lazy::new(|| {
    hostname::get()
        .unwrap_or("Unknown".into())
        .to_string_lossy()
        .to_string()
});

#[derive(PartialEq)]
pub enum BackType {
    Start,
    Stop,
    Back,
}

pub struct EmbyClient {
    pub url: Mutex<Option<Url>>,
    pub client: Client,
    pub semaphore: Arc<tokio::sync::Semaphore>,
    pub headers: Mutex<reqwest::header::HeaderMap>,
    pub user_id: Mutex<String>,
    pub user_name: Mutex<String>,
    pub user_password: Mutex<String>,
    pub user_access_token: Mutex<String>,
    pub server_name: Mutex<String>,
    pub server_name_hash: Mutex<String>,
}

fn generate_emby_authorization(
    user_id: &str, client: &str, device: &str, device_id: &str, version: &str,
) -> String {
    format!(
        "Emby UserId={},Client={},Device={},DeviceId={},Version={}",
        user_id, client, device, device_id, version
    )
}

static DOMAIN_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"(https?://)([^/]+)").unwrap());

fn generate_hash(s: &str) -> String {
    let mut hasher = fnv::FnvHasher::default();
    hasher.write(s.as_bytes());
    format!("{:x}", hasher.finish())
}

fn hide_domain(url: &str) -> String {
    let hidden = "\x1b[35mDomain Hidden\x1b[0m";
    DOMAIN_REGEX
        .replace(url, &format!("$1{}", hidden))
        .to_string()
}

impl EmbyClient {
    pub fn default() -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("Accept-Encoding", HeaderValue::from_static("gzip"));
        headers.insert(
            "x-emby-authorization",
            HeaderValue::from_str(&generate_emby_authorization(
                "",
                CLIENT_ID,
                &DEVICE_NAME,
                &DEVICE_ID,
                VERSION,
            ))
            .unwrap(),
        );
        Self {
            url: Mutex::new(None),
            client: ReqClient::build(),
            semaphore: Arc::new(tokio::sync::Semaphore::new(SETTINGS.threads() as usize)),
            headers: Mutex::new(headers),
            user_id: Mutex::new(String::new()),
            user_name: Mutex::new(String::new()),
            user_password: Mutex::new(String::new()),
            user_access_token: Mutex::new(String::new()),
            server_name: Mutex::new(String::new()),
            server_name_hash: Mutex::new(String::new()),
        }
    }

    pub fn init(&self, account: &Account) -> Result<(), Box<dyn std::error::Error>> {
        self.header_change_url(&account.server, &account.port)?;
        self.header_change_token(&account.access_token)?;
        self.set_user_id(&account.user_id)?;
        self.set_user_name(&account.username)?;
        self.set_user_password(&account.password)?;
        self.set_user_access_token(&account.access_token)?;
        self.set_server_name(&account.servername)?;
        crate::ui::provider::set_admin(false);
        spawn_tokio_without_await(async move {
            match EMBY_CLIENT.authenticate_admin().await {
                Ok(r) => {
                    if r.policy.is_administrator {
                        crate::ui::provider::set_admin(true);
                    }
                }
                Err(e) => warn!("Failed to authenticate as admin: {}", e),
            }
        });
        Ok(())
    }

    pub fn header_change_token(&self, token: &str) -> Result<()> {
        let mut headers = self
            .headers
            .lock()
            .map_err(|_| anyhow!("Failed to acquire lock on headers"))?;
        headers.insert("X-Emby-Token", HeaderValue::from_str(token)?);
        Ok(())
    }

    pub fn header_change_url(&self, url: &str, port: &str) -> Result<()> {
        let mut url = Url::parse(url)?;
        url.set_port(Some(port.parse::<u16>().unwrap_or_default()))
            .map_err(|_| anyhow!("Failed to set port"))?;
        let mut url_lock = self
            .url
            .lock()
            .map_err(|_| anyhow!("Failed to acquire lock on URL"))?;
        *url_lock = Some(url.join("emby/")?);
        Ok(())
    }

    pub fn set_user_id(&self, user_id: &str) -> Result<()> {
        let mut user_id_lock = self
            .user_id
            .lock()
            .map_err(|_| anyhow!("Failed to acquire lock on user_id"))?;
        *user_id_lock = user_id.to_string();
        self.header_change_user_id(user_id)?;
        Ok(())
    }

    pub fn header_change_user_id(&self, user_id: &str) -> Result<()> {
        let mut headers = self
            .headers
            .lock()
            .map_err(|_| anyhow!("Failed to acquire lock on headers"))?;
        headers.insert(
            "x-emby-authorization",
            HeaderValue::from_str(&generate_emby_authorization(
                user_id,
                CLIENT_ID,
                &DEVICE_NAME,
                &DEVICE_ID,
                VERSION,
            ))?,
        );
        Ok(())
    }

    pub fn set_user_name(&self, user_name: &str) -> Result<()> {
        let mut user_name_lock = self
            .user_name
            .lock()
            .map_err(|_| anyhow!("Failed to acquire lock on user_name"))?;
        *user_name_lock = user_name.to_string();
        Ok(())
    }

    pub fn set_user_password(&self, user_password: &str) -> Result<()> {
        let mut user_password_lock = self
            .user_password
            .lock()
            .map_err(|_| anyhow!("Failed to acquire lock on user_password"))?;
        *user_password_lock = user_password.to_string();
        Ok(())
    }

    pub fn set_user_access_token(&self, user_access_token: &str) -> Result<()> {
        let mut user_access_token_lock = self
            .user_access_token
            .lock()
            .map_err(|_| anyhow!("Failed to acquire lock on user_access_token"))?;
        *user_access_token_lock = user_access_token.to_string();
        Ok(())
    }

    pub fn set_server_name(&self, server_name: &str) -> Result<()> {
        let mut server_name_lock = self
            .server_name
            .lock()
            .map_err(|_| anyhow!("Failed to acquire lock on server_name"))?;
        *server_name_lock = server_name.to_string();

        let mut server_name_hash_lock = self
            .server_name_hash
            .lock()
            .map_err(|_| anyhow!("Failed to acquire lock on server_name_hash"))?;

        *server_name_hash_lock = generate_hash(server_name);
        Ok(())
    }

    pub fn get_url_and_headers(&self) -> Result<(Url, reqwest::header::HeaderMap)> {
        let url = self
            .url
            .lock()
            .map_err(|_| anyhow!("Failed to acquire lock on URL"))?
            .as_ref()
            .ok_or_else(|| anyhow!("URL is not set"))?
            .clone();
        let headers = self
            .headers
            .lock()
            .map_err(|_| anyhow!("Failed to acquire lock on headers"))?
            .clone();
        Ok((url, headers))
    }

    pub async fn request<T>(&self, path: &str, params: &[(&str, &str)]) -> Result<T>
    where
        T: for<'de> Deserialize<'de> + Send + 'static,
    {
        let request = self.prepare_request(Method::GET, path, params)?;
        let res = self.send_request(request).await?;

        let res = match res.error_for_status() {
            Ok(r) => r,
            Err(e) => {
                let Some(status) = e.status() else {
                    return Err(anyhow!("Failed to get status"));
                };
                return Err(anyhow!("{}", status));
            }
        };

        let res_text = res.text().await?;
        match serde_json::from_str(&res_text) {
            Ok(json) => Ok(json),
            Err(e) => Err(anyhow!(
                "Request Path: {}\nFailed parsing response to json {}: {}",
                path,
                e,
                res_text
            )),
        }
    }

    pub async fn request_picture(
        &self, path: &str, params: &[(&str, &str)], etag: Option<String>,
    ) -> Result<Response> {
        let request = self
            .prepare_request(Method::GET, path, params)?
            .header("If-None-Match", etag.unwrap_or_default());
        let res = request.send().await?;
        Ok(res)
    }

    pub async fn post<B>(&self, path: &str, params: &[(&str, &str)], body: B) -> Result<Response>
    where
        B: Serialize,
    {
        let request = self
            .prepare_request(Method::POST, path, params)?
            .json(&body);
        let res = self.send_request(request).await?;
        Ok(res)
    }

    pub async fn post_raw<B>(&self, path: &str, body: B, content_type: &str) -> Result<Response>
    where
        reqwest::Body: From<B>,
    {
        let request = self
            .prepare_request_headers(Method::POST, path, &[], content_type)?
            .body(body);
        let res = self.send_request(request).await?;
        Ok(res)
    }

    pub async fn post_json<B, T>(
        &self, path: &str, params: &[(&str, &str)], body: B,
    ) -> Result<T, anyhow::Error>
    where
        B: Serialize,
        T: DeserializeOwned,
    {
        let response = self.post(path, params, body).await?.error_for_status()?;
        let parsed = response.json::<T>().await?;
        Ok(parsed)
    }

    fn prepare_request(
        &self, method: Method, path: &str, params: &[(&str, &str)],
    ) -> Result<RequestBuilder> {
        let (mut url, headers) = self.get_url_and_headers()?;
        url = url.join(path)?;
        self.add_params_to_url(&mut url, params);
        Ok(self.client.request(method, url).headers(headers))
    }

    fn prepare_request_headers(
        &self, method: Method, path: &str, params: &[(&str, &str)], content_type: &str,
    ) -> Result<RequestBuilder> {
        let (mut url, mut headers) = self.get_url_and_headers()?;
        url = url.join(path)?;
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            HeaderValue::from_str(content_type)?,
        );
        self.add_params_to_url(&mut url, params);
        Ok(self.client.request(method, url).headers(headers))
    }

    async fn send_request(&self, request: RequestBuilder) -> Result<Response> {
        let permit = self.semaphore.acquire().await?;
        let res = match request.send().await {
            Ok(r) => r,
            Err(e) => return Err(anyhow!(e.to_user_facing())),
        };
        drop(permit);
        Ok(res)
    }

    pub async fn authenticate_admin(&self) -> Result<AuthenticateResponse> {
        let path = format!("Users/{}", self.user_id());
        let res = self.request(&path, &[]).await?;
        Ok(res)
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<LoginResponse> {
        let body = json!({
            "Username": username,
            "Pw": password
        });
        self.post_json("Users/authenticatebyname", &[], body).await
    }

    pub fn add_params_to_url(&self, url: &mut Url, params: &[(&str, &str)]) {
        for (key, value) in params {
            url.query_pairs_mut().append_pair(key, value);
        }
        debug!("Request URL: {}", hide_domain(url.as_str()));
    }

    // jellyfin
    pub fn get_direct_stream_url(
        &self, continer: &str, media_source_id: &str, etag: &str,
    ) -> String {
        let mut url = self.url.lock().unwrap().as_ref().unwrap().clone();
        url.path_segments_mut().unwrap().pop();
        let path = format!("Videos/{}/stream.{}", media_source_id, continer);
        let mut url = url.join(&path).unwrap();
        url.query_pairs_mut()
            .append_pair("Static", "true")
            .append_pair("mediaSourceId", media_source_id)
            .append_pair("deviceId", &DEVICE_ID)
            .append_pair("api_key", self.user_access_token.lock().unwrap().as_str())
            .append_pair("Tag", etag);
        url.to_string()
    }

    pub async fn search(&self, query: &str, filter: &[&str], start_index: &str) -> Result<List> {
        let filter_str = filter.join(",");
        let path = format!("Users/{}/Items", self.user_id());
        let params = [
            (
                "Fields",
                "BasicSyncInfo,CanDelete,PrimaryImageAspectRatio,ProductionYear,Status,EndDate,CommunityRating",
            ),
            ("IncludeItemTypes", &filter_str),
            ("IncludeSearchTypes", &filter_str),
            ("StartIndex", start_index),
            ("SortBy", "SortName"),
            ("SortOrder", "Ascending"),
            ("EnableImageTypes", "Primary,Backdrop,Thumb,Banner"),
            ("ImageTypeLimit", "1"),
            ("Recursive", "true"),
            ("SearchTerm", query),
            ("GroupProgramsBySeries", "true"),
            ("Limit", "50"),
        ];
        self.request(&path, &params).await
    }

    pub async fn get_episodes(&self, id: &str, season_id: &str) -> Result<List> {
        let path = format!("Shows/{}/Episodes", id);
        let params = [
            (
                "Fields",
                "Overview,PrimaryImageAspectRatio,PremiereDate,ProductionYear,SyncStatus",
            ),
            ("ImageTypeLimit", "1"),
            ("SeasonId", season_id),
            ("UserId", &self.user_id()),
        ];
        self.request(&path, &params).await
    }

    pub async fn get_item_info(&self, id: &str) -> Result<SimpleListItem> {
        let path = format!("Users/{}/Items/{}", self.user_id(), id);
        let params = [("Fields", "ShareLevel")];
        self.request(&path, &params).await
    }

    pub async fn get_edit_info(&self, id: &str) -> Result<SimpleListItem> {
        let path = format!("Users/{}/Items/{}", self.user_id(), id);
        let params = [("Fields", "ChannelMappingInfo")];
        self.request(&path, &params).await
    }

    pub async fn get_resume(&self) -> Result<List> {
        let path = format!("Users/{}/Items/Resume", self.user_id());
        let params = [
            ("Recursive", "true"),
            (
                "Fields",
                "Overview,BasicSyncInfo,CanDelete,PrimaryImageAspectRatio,ProductionYear,CommunityRating",
            ),
            ("EnableImageTypes", "Primary,Backdrop,Thumb,Banner"),
            ("ImageTypeLimit", "1"),
            ("MediaTypes", "Video"),
        ];
        self.request(&path, &params).await
    }

    pub async fn get_image_items(&self, id: &str) -> Result<Vec<ImageItem>> {
        let path = format!("Items/{}/Images", id);
        self.request(&path, &[]).await
    }

    pub async fn image_request(
        &self, id: &str, image_type: &str, tag: Option<u8>, etag: Option<String>,
    ) -> Result<Response> {
        let mut path = format!("Items/{}/Images/{}", id, image_type);
        if let Some(tag) = tag {
            path.push_str(&format!("/{}", tag));
        }
        let params = [
            (
                "maxHeight",
                if image_type == "Backdrop" {
                    "800"
                } else {
                    "300"
                },
            ),
            (
                "maxWidth",
                if image_type == "Backdrop" {
                    "1280"
                } else {
                    "300"
                },
            ),
        ];
        self.request_picture(&path, &params, etag).await
    }

    pub async fn get_image(&self, id: &str, image_type: &str, tag: Option<u8>) -> Result<String> {
        let mut path = emby_cache_path();
        path.push(format!("{}-{}-{}", id, image_type, tag.unwrap_or(0)));

        let mut etag: Option<String> = None;

        if path.exists() {
            #[cfg(target_os = "linux")]
            {
                etag = xattr::get(&path, "user.etag")
                    .ok()
                    .flatten()
                    .and_then(|v| String::from_utf8(v).ok());
            }
            #[cfg(target_os = "windows")]
            {
                etag = get_xattr(&path, "user.etag").ok();
            }
        }

        match self.image_request(id, image_type, tag, etag).await {
            Ok(response) => {
                if response.status().is_redirection() {
                    return Ok(path.to_string_lossy().to_string());
                } else if !response.status().is_success() {
                    return Err(anyhow!("Failed to get image: {}", response.status()));
                }

                let etag = response
                    .headers()
                    .get("ETag")
                    .map(|v| v.to_str().unwrap_or_default().to_string());

                let bytes = response.bytes().await?;

                let path = if bytes.len() > 1000 {
                    self.save_image(id, image_type, tag, &bytes, etag)
                } else {
                    String::new()
                };

                Ok(path)
            }
            Err(e) => Err(e),
        }
    }

    // Only support base64 encoded images
    pub async fn post_image<B>(
        &self, id: &str, image_type: &str, bytes: B, content_type: &str,
    ) -> Result<Response>
    where
        reqwest::Body: From<B>,
    {
        let path = format!("Items/{}/Images/{}", id, image_type);
        self.post_raw(&path, bytes, content_type)
            .await?
            .error_for_status()
            .map_err(|e| e.into())
    }

    pub async fn post_image_url(
        &self, id: &str, image_type: &str, tag: u8, url: &str,
    ) -> Result<Response> {
        let path = format!("Items/{}/Images/{}/{}", id, tag, image_type);
        let body = json!({ "Url": url });
        self.post(&path, &[], body).await
    }

    pub async fn delete_image(
        &self, id: &str, image_type: &str, tag: Option<u8>,
    ) -> Result<Response> {
        let mut path = format!("Items/{}/Images/{}", id, image_type);
        if let Some(tag) = tag {
            path.push_str(&format!("/{}", tag));
        }
        path.push_str("/Delete");
        self.post(&path, &[], json!({})).await
    }

    pub fn save_image(
        &self, id: &str, image_type: &str, tag: Option<u8>, bytes: &[u8], etag: Option<String>,
    ) -> String {
        let cache_path = emby_cache_path();
        let path = format!("{}-{}-{}", id, image_type, tag.unwrap_or(0));
        let path = cache_path.join(path);
        std::fs::write(&path, bytes).unwrap();
        if let Some(etag) = etag {
            #[cfg(target_os = "linux")]
            xattr::set(&path, "user.etag", etag.as_bytes()).unwrap_or_else(|e| {
                tracing::warn!("Failed to set etag xattr: {}", e);
            });
            #[cfg(target_os = "windows")]
            set_xattr(&path, "user.etag", etag).unwrap_or_else(|e| {
                tracing::warn!("Failed to set etag xattr: {}", e);
            });
        }
        path.to_string_lossy().to_string()
    }

    pub async fn get_artist_albums(&self, id: &str, artist_id: &str) -> Result<List> {
        let path = format!("Users/{}/Items", self.user_id());
        let params = [
            ("IncludeItemTypes", "MusicAlbum"),
            ("Recursive", "true"),
            ("ImageTypeLimit", "1"),
            ("Limit", "12"),
            ("SortBy", "ProductionYear,SortName"),
            ("EnableImageTypes", "Primary,Backdrop,Thumb,Banner"),
            ("SortOrder", "Descending"),
            (
                "Fields",
                "BasicSyncInfo,CanDelete,PrimaryImageAspectRatio,ProductionYear",
            ),
            ("AlbumArtistIds", artist_id),
            ("ExcludeItemIds", id),
        ];
        self.request(&path, &params).await
    }

    pub async fn get_shows_next_up(&self, series_id: &str) -> Result<List> {
        let path = "Shows/NextUp".to_string();
        let params = [
            ("Fields", "BasicSyncInfo,CanDelete,PrimaryImageAspectRatio"),
            ("Limit", "1"),
            ("ImageTypeLimit", "1"),
            ("SeriesId", series_id),
            ("UserId", &self.user_id()),
        ];
        self.request(&path, &params).await
    }

    pub async fn get_playbackinfo(&self, id: &str) -> Result<Media> {
        let path = format!("Items/{}/PlaybackInfo", id);
        let params = [
            ("StartTimeTicks", "0"),
            ("UserId", &self.user_id()),
            ("AutoOpenLiveStream", "true"),
            ("IsPlayback", "true"),
            ("AudioStreamIndex", "1"),
            ("SubtitleStreamIndex", "1"),
            ("MaxStreamingBitrate", "2147483647"),
            ("reqformat", "json"),
        ];
        let profile: Value = serde_json::from_str(PROFILE).expect("Failed to parse profile");
        self.post_json(&path, &params, profile).await
    }

    pub async fn scan(&self, id: &str) -> Result<Response> {
        let path = format!("Items/{}/Refresh", id);
        let params = [
            ("Recursive", "true"),
            ("ImageRefreshMode", "Default"),
            ("MetadataRefreshMode", "Default"),
            ("ReplaceAllImages", "false"),
            ("ReplaceAllMetadata", "false"),
        ];
        self.post(&path, &params, json!({})).await
    }

    pub async fn fullscan(
        &self, id: &str, replace_images: &str, replace_metadata: &str,
    ) -> Result<Response> {
        let path = format!("Items/{}/Refresh", id);
        let params = [
            ("Recursive", "true"),
            ("ImageRefreshMode", "FullRefresh"),
            ("MetadataRefreshMode", "FullRefresh"),
            ("ReplaceAllImages", replace_images),
            ("ReplaceAllMetadata", replace_metadata),
        ];
        self.post(&path, &params, json!({})).await
    }

    pub async fn remote_search(
        &self, type_: &str, info: &RemoteSearchInfo,
    ) -> Result<Vec<RemoteSearchResult>> {
        let path = format!("Items/RemoteSearch/{}", type_);
        let body = json!(info);
        self.post_json(&path, &[], body).await
    }

    pub async fn get_user_avatar(&self) -> Result<String> {
        let path = format!("Users/{}/Images/Primary", self.user_id());
        let params = [("maxHeight", "50"), ("maxWidth", "50")];
        let response = self.request_picture(&path, &params, None).await?;
        let etag = response
            .headers()
            .get("ETag")
            .map(|v| v.to_str().unwrap_or_default().to_string());
        let bytes = response.bytes().await?;
        let path = self.save_image(&self.user_id(), "Primary", None, &bytes, etag);
        Ok(path)
    }

    pub async fn get_external_id_info(&self, id: &str) -> Result<Vec<ExternalIdInfo>> {
        let path = format!("Items/{}/ExternalIdInfos", id);
        let params = [("IsSupportedAsIdentifier", "true")];
        self.request(&path, &params).await
    }

    pub async fn get_live_playbackinfo(&self, id: &str) -> Result<LiveMedia> {
        let path = format!("Items/{}/PlaybackInfo", id);
        let params = [
            ("StartTimeTicks", "0"),
            ("UserId", &self.user_id()),
            ("AutoOpenLiveStream", "false"),
            ("IsPlayback", "false"),
            ("MaxStreamingBitrate", "2147483647"),
            ("reqformat", "json"),
        ];
        let profile: Value = serde_json::from_str(PROFILE).unwrap();
        self.post_json(&path, &params, profile).await
    }

    pub async fn get_sub(&self, id: &str, source_id: &str) -> Result<Media> {
        let path = format!("Items/{}/PlaybackInfo", id);
        let params = [
            ("StartTimeTicks", "0"),
            ("UserId", &self.user_id()),
            ("AutoOpenLiveStream", "true"),
            ("IsPlayback", "true"),
            ("AudioStreamIndex", "1"),
            ("SubtitleStreamIndex", "1"),
            ("MediaSourceId", source_id),
            ("MaxStreamingBitrate", "4000000"),
            ("reqformat", "json"),
        ];
        let profile: Value = serde_json::from_str(PROFILE).unwrap();
        self.post_json(&path, &params, profile).await
    }

    pub async fn get_library(&self) -> Result<List> {
        let path = format!("Users/{}/Views", &self.user_id());
        self.request(&path, &[]).await
    }

    pub async fn get_latest(&self, id: &str) -> Result<Vec<SimpleListItem>> {
        let path = format!("Users/{}/Items/Latest", &self.user_id());
        let params = [
            ("Limit", "16"),
            (
                "Fields",
                "BasicSyncInfo,CanDelete,PrimaryImageAspectRatio,ProductionYear,CommunityRating",
            ),
            ("ParentId", id),
            ("ImageTypeLimit", "1"),
            ("EnableImageTypes", "Primary,Backdrop,Thumb,Banner"),
        ];
        self.request(&path, &params).await
    }

    pub fn get_streaming_url(&self, path: &str) -> String {
        let url = self.url.lock().unwrap().as_ref().unwrap().clone();
        url.join(path.trim_start_matches('/')).unwrap().to_string()
    }

    pub async fn get_list(
        &self, id: &str, start: u32, include_item_types: &str, list_type: ListType,
        sort_order: &str, sortby: &str,
    ) -> Result<List> {
        let user_id = &self.user_id();
        let path = match list_type {
            ListType::All => format!("Users/{}/Items", user_id),
            ListType::Resume => format!("Users/{}/Items/Resume", user_id),
            ListType::Genres => "Genres".to_string(),
            _ => format!("Users/{}/Items", user_id),
        };
        let include_item_type = match list_type {
            ListType::Tags => "Tag",
            ListType::BoxSet => "BoxSet",
            _ => include_item_types,
        };
        let start_string = start.to_string();
        let params = match list_type {
            ListType::All | ListType::Liked | ListType::Tags | ListType::BoxSet => {
                vec![
                    ("Limit", "50"),
                    (
                        "Fields",
                        "Overview,BasicSyncInfo,CanDelete,PrimaryImageAspectRatio,ProductionYear,Status,EndDate,CommunityRating",
                    ),
                    ("ParentId", id),
                    ("ImageTypeLimit", "1"),
                    ("StartIndex", &start_string),
                    ("Recursive", "true"),
                    ("IncludeItemTypes", include_item_type),
                    ("SortBy", sortby),
                    ("SortOrder", sort_order),
                    ("EnableImageTypes", "Primary,Backdrop,Thumb,Banner"),
                    if list_type == ListType::Liked {("Filters", "IsFavorite")} else {("", "")},
                ]
            }
            ListType::Resume => {
                vec![
                    (
                        "Fields",
                        "Overview,BasicSyncInfo,CanDelete,PrimaryImageAspectRatio,ProductionYear",
                    ),
                    ("ParentId", id),
                    ("EnableImageTypes", "Primary,Backdrop,Thumb,Banner"),
                    ("ImageTypeLimit", "1"),
                    (
                        "IncludeItemTypes",
                        match include_item_type {
                            "Series" => "Episode",
                            _ => include_item_type,
                        },
                    ),
                    ("Limit", "30"),
                ]
            }
            ListType::Genres => vec![
                ("Fields", "BasicSyncInfo,CanDelete,PrimaryImageAspectRatio"),
                ("IncludeItemTypes", include_item_type),
                ("StartIndex", &start_string),
                ("ImageTypeLimit", "1"),
                ("EnableImageTypes", "Primary,Backdrop,Thumb,Banner"),
                ("Limit", "50"),
                ("userId", user_id),
                ("Recursive", "true"),
                ("ParentId", id),
            ],
            _ => vec![],
        };
        self.request(&path, &params).await
    }

    pub async fn get_inlist(
        &self, id: Option<String>, start: u32, listtype: &str, parentid: &str, sort_order: &str,
        sortby: &str,
    ) -> Result<List> {
        let path = format!("Users/{}/Items", &self.user_id());
        let start_string = start.to_string();
        let mut params = vec![
            ("Limit", "50"),
            (
                "Fields",
                "Overview,BasicSyncInfo,CanDelete,PrimaryImageAspectRatio,ProductionYear,Status,EndDate,CommunityRating",
            ),
            ("ImageTypeLimit", "1"),
            ("StartIndex", &start_string),
            ("Recursive", "true"),
            ("IncludeItemTypes", "Movie,Series,MusicAlbum"),
            ("SortBy", sortby),
            ("SortOrder", sort_order),
            ("EnableImageTypes", "Primary,Backdrop,Thumb,Banner"),
            if listtype == "Genres" || listtype == "Genre" {
                ("GenreIds", parentid)
            } else if listtype == "Studios" {
                ("StudioIds", parentid)
            } else {
                ("TagIds", parentid)
            },
        ];
        let id_clone;
        if let Some(id) = id {
            id_clone = id.clone();
            params.push(("ParentId", &id_clone));
        }
        self.request(&path, &params).await
    }

    pub async fn like(&self, id: &str) -> Result<()> {
        let path = format!(
            "Users/{}/FavoriteItems/{}",
            &self.user_id.lock().unwrap(),
            id
        );
        self.post(&path, &[], json!({})).await?;
        Ok(())
    }

    pub async fn unlike(&self, id: &str) -> Result<()> {
        let path = format!(
            "Users/{}/FavoriteItems/{}/Delete",
            &self.user_id.lock().unwrap(),
            id
        );
        self.post(&path, &[], json!({})).await?;
        Ok(())
    }

    pub async fn set_as_played(&self, id: &str) -> Result<()> {
        let path = format!("Users/{}/PlayedItems/{}", &self.user_id(), id);
        self.post(&path, &[], json!({})).await?;
        Ok(())
    }

    pub async fn set_as_unplayed(&self, id: &str) -> Result<()> {
        let path = format!(
            "Users/{}/PlayedItems/{}/Delete",
            &self.user_id.lock().unwrap(),
            id
        );
        self.post(&path, &[], json!({})).await?;
        Ok(())
    }

    pub async fn position_back(&self, back: &Back, backtype: BackType) -> Result<()> {
        let path = match backtype {
            BackType::Start => "Sessions/Playing".to_string(),
            BackType::Stop => "Sessions/Playing/Stopped".to_string(),
            BackType::Back => "Sessions/Playing/Progress".to_string(),
        };
        let params = [("reqformat", "json")];
        let body = json!({"VolumeLevel":100,"NowPlayingQueue":[],"IsMuted":false,"IsPaused":false,"MaxStreamingBitrate":2147483647,"RepeatMode":"RepeatNone","PlaybackStartTimeTicks":back.start_tick,"SubtitleOffset":0,"PlaybackRate":1,"PositionTicks":back.tick,"PlayMethod":"DirectStream","PlaySessionId":back.playsessionid,"MediaSourceId":back.mediasourceid,"PlaylistIndex":0,"PlaylistLength":1,"CanSeek":true,"ItemId":back.id,"Shuffle":false});
        self.post(&path, &params, body).await?;
        Ok(())
    }

    pub async fn get_similar(&self, id: &str) -> Result<List> {
        let path = format!("Items/{}/Similar", id);
        let params = [
            (
                "Fields",
                "BasicSyncInfo,CanDelete,PrimaryImageAspectRatio,ProductionYear,Status,EndDate,CommunityRating",
            ),
            ("UserId", &self.user_id()),
            ("ImageTypeLimit", "1"),
            ("Limit", "12"),
        ];
        self.request(&path, &params).await
    }

    pub async fn get_actor_item_list(&self, id: &str, types: &str) -> Result<List> {
        let path = format!("Users/{}/Items", &self.user_id());
        let params = [
            (
                "Fields",
                "PrimaryImageAspectRatio,ProductionYear,CommunityRating",
            ),
            ("PersonIds", id),
            ("Recursive", "true"),
            ("CollapseBoxSetItems", "false"),
            ("SortBy", "SortName"),
            ("SortOrder", "Ascending"),
            ("IncludeItemTypes", types),
            ("ImageTypeLimit", "1"),
            ("Limit", "12"),
        ];
        self.request(&path, &params).await
    }

    pub async fn get_person_large_list(
        &self, id: &str, types: &str, sort_by: &str, sort_order: &str, start_index: u32,
    ) -> Result<List> {
        let start_string = start_index.to_string();
        let path = format!("Users/{}/Items", &self.user_id());
        let params = [
            (
                "Fields",
                "Overview,PrimaryImageAspectRatio,ProductionYear,CommunityRating",
            ),
            ("PersonIds", id),
            ("Recursive", "true"),
            ("CollapseBoxSetItems", "false"),
            ("SortBy", sort_by),
            ("SortOrder", sort_order),
            ("IncludeItemTypes", types),
            ("StartIndex", &start_string),
            ("ImageTypeLimit", "1"),
            ("Limit", "50"),
        ];
        self.request(&path, &params).await
    }

    pub async fn get_continue_play_list(&self, parent_id: &str) -> Result<List> {
        let path = "Shows/NextUp".to_string();
        let params = [
            (
                "Fields",
                "BasicSyncInfo,CanDelete,PrimaryImageAspectRatio,Overview",
            ),
            ("Limit", "40"),
            ("ImageTypeLimit", "1"),
            ("SeriesId", parent_id),
            ("UserId", &self.user_id()),
        ];
        self.request(&path, &params).await
    }

    pub async fn get_season_list(&self, parent_id: &str) -> Result<List> {
        let path = format!("Shows/{}/Seasons", parent_id);
        let params = [
            (
                "Fields",
                "BasicSyncInfo,CanDelete,PremiereDate,PrimaryImageAspectRatio,Overview",
            ),
            ("UserId", &self.user_id()),
            ("ImageTypeLimit", "1"),
        ];
        self.request(&path, &params).await
    }

    pub async fn get_search_recommend(&self) -> Result<List> {
        let path = format!("Users/{}/Items", &self.user_id());
        let params = [
            ("Limit", "20"),
            ("EnableTotalRecordCount", "false"),
            ("ImageTypeLimit", "0"),
            ("Recursive", "true"),
            ("IncludeItemTypes", "Movie,Series"),
            ("SortBy", "IsFavoriteOrLiked,Random"),
            ("Recursive", "true"),
        ];
        self.request(&path, &params).await
    }

    pub async fn get_favourite(
        &self, types: &str, start: u32, limit: u32, sort_by: &str, sort_order: &str,
    ) -> Result<List> {
        let user_id = {
            let user_id = self.user_id.lock().unwrap();
            user_id.to_owned()
        };
        let path = if types == "People" {
            "Persons".to_string()
        } else {
            format!("Users/{}/Items", user_id)
        };
        let params = [
            (
                "Fields",
                "BasicSyncInfo,CanDelete,PrimaryImageAspectRatio,ProductionYear,CommunityRating",
            ),
            ("Filters", "IsFavorite"),
            ("Recursive", "true"),
            ("CollapseBoxSetItems", "false"),
            ("SortBy", sort_by),
            ("SortOrder", sort_order),
            ("IncludeItemTypes", types),
            ("Limit", &limit.to_string()),
            ("StartIndex", &start.to_string()),
            if types == "People" {
                ("UserId", &user_id)
            } else {
                ("", "")
            },
        ];
        self.request(&path, &params).await
    }

    pub async fn get_included(&self, id: &str) -> Result<List> {
        let path = format!("Users/{}/Items", &self.user_id());
        let params = [
            (
                "Fields",
                "BasicSyncInfo,CanDelete,PrimaryImageAspectRatio,CommunityRating",
            ),
            ("Limit", "12"),
            ("ListItemIds", id),
            ("Recursive", "true"),
            ("IncludeItemTypes", "Playlist,BoxSet"),
            ("SortBy", "SortName"),
            ("Recursive", "true"),
        ];
        self.request(&path, &params).await
    }

    pub async fn get_includedby(&self, parent_id: &str) -> Result<List> {
        let path = format!("Users/{}/Items", &self.user_id());
        let params = [
            (
                "Fields",
                "BasicSyncInfo,CanDelete,PrimaryImageAspectRatio,ProductionYear,Status,EndDate,CommunityRating",
            ),
            ("ImageTypeLimit", "1"),
            ("ParentId", parent_id),
            ("SortBy", "DisplayOrder"),
            ("SortOrder", "Ascending"),
            ("EnableTotalRecordCount", "false"),
        ];
        self.request(&path, &params).await
    }

    pub async fn get_folder_include(
        &self, parent_id: &str, sort_by: &str, sort_order: &str, start_index: u32,
    ) -> Result<List> {
        let path = format!("Users/{}/Items", &self.user_id());
        let start_index_string = start_index.to_string();
        let sort_by = format!("IsFolder,{}", sort_by);
        let params = [
            (
                "Fields",
                "BasicSyncInfo,CanDelete,PrimaryImageAspectRatio,ProductionYear,Status,EndDate,CommunityRating",
            ),
            ("StartIndex", &start_index_string),
            ("ImageTypeLimit", "1"),
            ("Limit", "50"),
            ("ParentId", parent_id),
            ("SortBy", &sort_by),
            ("SortOrder", sort_order),
            ("EnableTotalRecordCount", "true"),
        ];
        self.request(&path, &params).await
    }

    pub async fn change_password(&self, new_password: &str) -> Result<()> {
        let path = format!("Users/{}/Password", &self.user_id());

        let old_password = match self.user_password.lock() {
            Ok(guard) => guard.to_string(),
            Err(_) => return Err(anyhow::anyhow!("Failed to acquire lock on user password")),
        };

        let body = json!({
            "CurrentPw": old_password,
            "NewPw": new_password
        });

        self.post(&path, &[], body).await?;
        Ok(())
    }

    pub async fn hide_from_resume(&self, id: &str) -> Result<()> {
        let path = format!("Users/{}/Items/{}/HideFromResume", &self.user_id(), id);
        let params = [("Hide", "true")];
        self.post(&path, &params, json!({})).await?;
        Ok(())
    }

    pub async fn get_songs(&self, parent_id: &str) -> Result<List> {
        let path = format!("Users/{}/Items", &self.user_id());
        let params = [
            (
                "Fields",
                "BasicSyncInfo,CanDelete,PrimaryImageAspectRatio,SyncStatus",
            ),
            ("ImageTypeLimit", "1"),
            ("ParentId", parent_id),
            ("EnableTotalRecordCount", "false"),
        ];
        self.request(&path, &params).await
    }

    pub fn get_song_streaming_uri(&self, id: &str) -> String {
        let url = self.url.lock().unwrap().as_ref().unwrap().clone();

        url.join(&format!("Audio/{}/universal?UserId={}&DeviceId={}&MaxStreamingBitrate=4000000&Container=opus,mp3|mp3,mp2,mp3|mp2,m4a|aac,mp4|aac,flac,webma,webm,wav|PCM_S16LE,wav|PCM_S24LE,ogg&TranscodingContainer=aac&TranscodingProtocol=hls&AudioCodec=aac&api_key={}&PlaySessionId=1715006733496&StartTimeTicks=0&EnableRedirection=true&EnableRemoteMedia=false",
        id, &self.user_id(), &DEVICE_ID.to_string(), self.user_access_token.lock().unwrap(), )).unwrap().to_string()
    }

    fn user_id(&self) -> String {
        self.user_id.lock().unwrap().to_string()
    }

    pub async fn get_additional(&self, id: &str) -> Result<List> {
        let path = format!("Videos/{}/AdditionalParts", id);
        let params: [(&str, &str); 1] = [("UserId", &self.user_id())];
        self.request(&path, &params).await
    }

    pub async fn get_channels(&self) -> Result<List> {
        let params = [
            ("IsAiring", "true"),
            ("userId", &self.user_id()),
            ("ImageTypeLimit", "1"),
            ("Limit", "12"),
            ("Fields", "ProgramPrimaryImageAspectRatio"),
            ("SortBy", "DefaultChannelOrder"),
            ("SortOrder", "Ascending"),
        ];
        self.request("LiveTv/Channels", &params).await
    }

    pub async fn get_channels_list(&self, start_index: u32) -> Result<List> {
        let params = [
            ("IsAiring", "true"),
            ("userId", &self.user_id()),
            ("ImageTypeLimit", "1"),
            ("Limit", "50"),
            ("Fields", "ProgramPrimaryImageAspectRatio"),
            ("SortBy", "DefaultChannelOrder"),
            ("SortOrder", "Ascending"),
            ("StartIndex", &start_index.to_string()),
        ];
        self.request("LiveTv/Channels", &params).await
    }

    pub async fn get_server_info(&self) -> Result<ServerInfo> {
        self.request("System/Info", &[]).await
    }

    pub async fn get_server_info_public(&self) -> Result<PublicServerInfo> {
        self.request("System/Info/Public", &[]).await
    }

    pub async fn shut_down(&self) -> Result<Response> {
        self.post("System/Shutdown", &[], json!({})).await
    }

    pub async fn restart(&self) -> Result<Response> {
        self.post("System/Restart", &[], json!({})).await
    }

    pub async fn get_activity_log(&self, has_user_id: bool) -> Result<ActivityLogs> {
        let params = [
            ("Limit", "15"),
            ("StartIndex", "0"),
            ("hasUserId", &has_user_id.to_string()),
        ];
        self.request("System/ActivityLog/Entries", &params).await
    }

    pub async fn get_scheduled_tasks(&self) -> Result<Vec<ScheduledTask>> {
        self.request("ScheduledTasks", &[]).await
    }

    pub async fn run_scheduled_task(&self, id: String) -> Result<()> {
        let path = format!("ScheduledTasks/Running/{}", &id);
        self.post(&path, &[], json!({})).await?;
        Ok(())
    }

    pub fn get_image_path(&self, id: &str, image_type: &str, image_index: Option<u32>) -> String {
        let path = format!("Items/{}/Images/{}/", id, image_type);
        let url = self
            .url
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone()
            .join(&path)
            .unwrap();
        match image_index {
            Some(index) => url.join(&index.to_string()).unwrap().to_string(),
            None => url.to_string(),
        }
    }

    pub async fn get_remote_image_list(
        &self, id: &str, start_index: u32, include_all_languages: bool, type_: &str,
        provider_name: &str,
    ) -> Result<ImageSearchResult> {
        let path = format!("Items/{}/RemoteImages", id);
        let start_string = start_index.to_string();
        let params = [
            ("Limit", "50"),
            ("StartIndex", &start_string),
            ("Type", type_),
            ("IncludeAllLanguages", &include_all_languages.to_string()),
            ("ProviderName", provider_name),
        ];

        self.request(&path, &params).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::error::UserFacingError;

    #[tokio::test]
    async fn search() {
        let _ = EMBY_CLIENT.header_change_url("https://example.com", "443");
        let result = EMBY_CLIENT.login("test", "test").await;
        match result {
            Ok(response) => {
                println!("{}", response.access_token);
                let _ = EMBY_CLIENT.header_change_token(&response.access_token);
                let _ = EMBY_CLIENT.set_user_id(&response.user.id);
            }
            Err(e) => {
                eprintln!("{}", e.to_user_facing());
            }
        }

        let result = EMBY_CLIENT.search("你的名字", &["Movie"], "0");
        match result.await {
            Ok(items) => {
                for item in items.items {
                    println!("{}", item.name);
                }
            }
            Err(e) => {
                eprintln!("{}", e.to_user_facing());
            }
        }
    }

    #[test]
    fn parse_url() {
        let uri = "127.0.0.1";
        let url = if Url::parse(uri).is_err() {
            format!("http://{}", uri)
        } else {
            uri.to_string()
        };

        assert_eq!(url, "http://127.0.0.1");
    }

    #[tokio::test]
    async fn test_upload_image() {
        let _ = EMBY_CLIENT.header_change_url("http://127.0.0.1", "8096");
        let result = EMBY_CLIENT.login("inaha", "").await;
        match result {
            Ok(response) => {
                println!("{}", response.access_token);
                let account = Account {
                    servername: "test".to_string(),
                    server: "http://127.0.0.1".to_string(),
                    username: "inaha".to_string(),
                    password: String::new(),
                    port: "8096".to_string(),
                    user_id: response.user.id,
                    access_token: response.access_token,
                    server_type: Some("Emby".to_string()),
                };
                let _ = EMBY_CLIENT.init(&account);
            }
            Err(e) => {
                eprintln!("{}", e.to_user_facing());
            }
        }

        let image = std::fs::read("/home/inaha/Works/tsukimi/target/debug/test.jpg").unwrap();
        use base64::{
            engine::general_purpose::STANDARD,
            Engine as _,
        };
        let image = STANDARD.encode(&image);
        match EMBY_CLIENT
            .post_image("293", "Thumb", image, "image/jpeg")
            .await
        {
            Ok(_) => {
                println!("success");
            }
            Err(e) => {
                eprintln!("{}", e.to_user_facing());
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn get_xattr(path: &std::path::Path, attr_name: &str) -> Result<String> {
    use std::{
        ffi::OsStr,
        io,
        os::windows::ffi::OsStrExt,
        str,
    };
    use windows::{
        core::{
            Error,
            PCWSTR,
        },
        Win32::{
            Foundation::{
                CloseHandle,
                INVALID_HANDLE_VALUE,
            },
            Storage::FileSystem::{
                CreateFileW,
                GetFileInformationByHandle,
                ReadFile,
                BY_HANDLE_FILE_INFORMATION,
                FILE_ATTRIBUTE_NORMAL,
                OPEN_EXISTING,
            },
        },
    };

    let stream_name = format!(":{}$DATA", attr_name);
    let full_path = format!("{}\\{}", path.display(), stream_name);

    let wide_path: Vec<u16> = OsStr::new(&full_path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let wide_path_pcwstr = PCWSTR::from_raw(wide_path.as_ptr());

    unsafe {
        let handle = CreateFileW(
            wide_path_pcwstr,
            2147483648u32,
            windows::Win32::Storage::FileSystem::FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )?;

        if handle == INVALID_HANDLE_VALUE {
            let err = Error::from(io::Error::last_os_error());
            if err.code().0 as u32 == 2 {
                return Err(anyhow!(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Attribute {} not found", attr_name),
                )));
            }
            return Err(anyhow!(err));
        }

        let mut file_info = BY_HANDLE_FILE_INFORMATION::default();
        GetFileInformationByHandle(handle, &mut file_info)?;

        let file_size = (file_info.nFileSizeHigh as u64) << 32 | (file_info.nFileSizeLow as u64);

        let mut buffer = vec![0u8; file_size as usize];
        let mut bytes_read: u32 = 0;

        ReadFile(handle, Some(&mut buffer), Some(&mut bytes_read), None)?;
        CloseHandle(handle)?;

        if bytes_read != file_size as u32 {
            return Err(anyhow!(io::Error::new(
                io::ErrorKind::Other,
                "Failed to read entire stream",
            )));
        }

        match str::from_utf8(&buffer) {
            Ok(s) => Ok(s.to_string()),
            Err(_) => Err(anyhow!(io::Error::new(
                io::ErrorKind::InvalidData,
                "Stream data is not valid UTF-8",
            ))),
        }
    }
}

#[cfg(target_os = "windows")]
fn set_xattr(path: &std::path::Path, attr_name: &str, value: String) -> Result<()> {
    use std::{
        ffi::OsStr,
        io,
        os::windows::ffi::OsStrExt,
    };
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{
                CloseHandle,
                INVALID_HANDLE_VALUE,
            },
            Storage::FileSystem::{
                CreateFileW,
                WriteFile,
                CREATE_ALWAYS,
                FILE_ATTRIBUTE_NORMAL,
            },
        },
    };

    let stream_name = format!(":{}$DATA", attr_name);
    let full_path = format!("{}\\{}", path.display(), stream_name);

    let wide_path: Vec<u16> = OsStr::new(&full_path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let wide_path_pcwstr = PCWSTR::from_raw(wide_path.as_ptr());

    unsafe {
        let handle = CreateFileW(
            wide_path_pcwstr,
            1073741824u32,
            windows::Win32::Storage::FileSystem::FILE_SHARE_MODE(0),
            None,
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )?;

        if handle == INVALID_HANDLE_VALUE {
            return Err(anyhow!(io::Error::last_os_error()));
        }

        let buffer = value.as_bytes();
        let mut bytes_written: u32 = 0;

        WriteFile(handle, Some(buffer), Some(&mut bytes_written), None)?;
        CloseHandle(handle)?;

        if bytes_written != buffer.len() as u32 {
            return Err(anyhow!(io::Error::new(
                io::ErrorKind::Other,
                "Failed to write entire stream",
            )));
        }

        Ok(())
    }
}
