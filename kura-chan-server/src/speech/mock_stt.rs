use std::future::Future;
use std::pin::Pin;

use super::SpeechToText;

pub struct MockStt;

impl SpeechToText for MockStt {
    fn transcribe<'a>(
        &'a self,
        audio: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<String, Box<dyn std::error::Error + Send + Sync>>> + Send + 'a>>
    {
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            tracing::debug!(audio_bytes = audio.len(), "Mock STT transcribing");
            Ok("你好，我是测试语音输入".into())
        })
    }
}
