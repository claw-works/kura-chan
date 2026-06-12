pub mod mock_stt;
pub mod mock_tts;

use std::future::Future;
use std::pin::Pin;

pub trait SpeechToText: Send + Sync {
    fn transcribe<'a>(
        &'a self,
        audio: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<String, Box<dyn std::error::Error + Send + Sync>>> + Send + 'a>>;
}

pub trait TextToSpeech: Send + Sync {
    fn synthesize<'a>(
        &'a self,
        text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>> + Send + 'a>>;
}
