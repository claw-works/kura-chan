pub mod mock_stt;
pub mod mock_tts;

#[allow(async_fn_in_trait)]
pub trait SpeechToText: Send + Sync {
    async fn transcribe(&self, audio: &[u8]) -> Result<String, Box<dyn std::error::Error + Send + Sync>>;
}

#[allow(async_fn_in_trait)]
pub trait TextToSpeech: Send + Sync {
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>;
}
