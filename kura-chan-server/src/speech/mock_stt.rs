use super::SpeechToText;

pub struct MockStt;

impl SpeechToText for MockStt {
    async fn transcribe(&self, audio: &[u8]) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        tracing::debug!(audio_bytes = audio.len(), "Mock STT transcribing");
        Ok("你好，我是测试语音输入".into())
    }
}
