// Her müzik servisi bu arayüzle bağlanır. Arayüz kaynaktan habersizdir.

pub mod soundcloud;
pub mod ytm;

use crate::model::{Section, Source, TrackInfo};

pub type Res<T> = Result<T, String>;

pub trait Provider: Send + Sync {
    fn source(&self) -> Source;
    fn search(&self, query: &str, limit: usize) -> Res<Vec<TrackInfo>>;
    fn home(&self) -> Res<Vec<Section>>;
    /// mpv'nin açabileceği adres (yt-dlp kancası akışı çözer).
    fn stream_url(&self, track: &TrackInfo) -> Res<String>;
}
