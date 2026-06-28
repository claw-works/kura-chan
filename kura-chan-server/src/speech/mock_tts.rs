use std::future::Future;
use std::pin::Pin;

use super::TextToSpeech;

pub struct MockTts;

impl TextToSpeech for MockTts {
    fn synthesize<'a>(
        &'a self,
        text: &'a str,
        _voice: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>> + Send + 'a>>
    {
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            tracing::debug!(text = %text, "Mock TTS synthesizing");
            Ok(vec![0u8; 1280])
        })
    }
}
