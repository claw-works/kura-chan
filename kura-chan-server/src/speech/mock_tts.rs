use super::TextToSpeech;

pub struct MockTts;

impl TextToSpeech for MockTts {
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        tracing::debug!(text = %text, "Mock TTS synthesizing");
        Ok(vec![0u8; 1280])
    }
}
