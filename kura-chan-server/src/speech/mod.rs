pub mod mock_stt;
pub mod mock_tts;
pub mod volc;

use std::future::Future;
use std::pin::Pin;

pub trait SpeechToText: Send + Sync {
    fn transcribe<'a>(
        &'a self,
        audio: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<String, Box<dyn std::error::Error + Send + Sync>>> + Send + 'a>>;
}

pub trait TextToSpeech: Send + Sync {
    /// Synthesize `text`. `voice` is the actor's voice spec ("provider/voiceid");
    /// pass "" to use the provider's default voice.
    fn synthesize<'a>(
        &'a self,
        text: &'a str,
        voice: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>> + Send + 'a>>;
}

/// Extract the voice id (TTS speaker) from a "provider/voiceid" spec.
pub fn voice_id(spec: &str) -> &str {
    spec.rsplit_once('/').map(|(_, v)| v).unwrap_or(spec)
}
