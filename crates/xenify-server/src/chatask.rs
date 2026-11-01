//! The server's AI for Chat's "Ask AI": whether someone may use it (an AI
//! is set up and their roles allow writing with AI), and answers. Chat
//! chooses what it's given: only the asker's own messages.

use futures_util::future::BoxFuture;
use serde_json::Value;

pub(crate) struct ChatAi {
    pub pool: deadpool_postgres::Pool,
}

impl xenify_chat_server::Assistant for ChatAi {
    fn ready(&self, user: i64) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            let Ok(c) = self.pool.get().await else { return false };
            let a = crate::ai::settings(&c).await;
            if a.url.is_empty() || a.text_model.is_empty() {
                return false;
            }
            c.query_one("SELECT xenify_can($1::BIGINT, 'ai.text')", &[&user]).await.is_ok_and(|r| r.get::<_, bool>(0))
        })
    }

    fn answer(&self, _user: i64, messages: Vec<Value>) -> BoxFuture<'static, Result<String, String>> {
        let pool = self.pool.clone();
        Box::pin(async move {
            let c = pool.get().await.map_err(|e| e.to_string())?;
            crate::ai::complete(&c, messages).await.unwrap_or_else(|| Err("This server has no AI.".into()))
        })
    }
}
