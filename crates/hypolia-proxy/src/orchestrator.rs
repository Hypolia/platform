use std::time::Duration;

use futures_util::StreamExt;
use hypolia_protocol::social::{
    FriendOutcome, FriendRequest, FriendsList, Party, PartyOutcome, PartyRequest,
};
use hypolia_protocol::{
    PlayerProfile, ProxyDisconnectRequest, ProxyEvent, ProxyStateSnapshot, TransferAckRequest,
};
use reqwest::{Client, RequestBuilder};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum OrchestratorError {
    #[error("orchestrator unreachable: {0}")]
    Http(#[from] reqwest::Error),
    #[error("orchestrator answered HTTP {0}")]
    Status(u16),
}

#[derive(Clone)]
pub struct Orchestrator {
    http: Client,
    base: String,
    token: Option<String>,
    version: String,
}

impl Orchestrator {
    pub fn new(base: &str, token: Option<String>, version: String) -> Self {
        Self {
            http: Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
            base: base.trim_end_matches('/').to_owned(),
            token,
            version,
        }
    }

    fn request(&self, builder: RequestBuilder) -> RequestBuilder {
        let builder = builder.header("x-hypolia-version", &self.version);
        match &self.token {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        }
    }

    pub async fn state(&self) -> Result<ProxyStateSnapshot, OrchestratorError> {
        let response = self
            .request(self.http.get(format!("{}/api/v1/proxy/state", self.base)))
            .timeout(Duration::from_secs(4))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(OrchestratorError::Status(response.status().as_u16()));
        }
        Ok(response.json().await?)
    }

    pub async fn profile(&self, player: Uuid) -> Result<PlayerProfile, OrchestratorError> {
        let response = self
            .request(
                self.http
                    .get(format!("{}/api/v1/players/{player}/profile", self.base)),
            )
            .timeout(Duration::from_secs(4))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(OrchestratorError::Status(response.status().as_u16()));
        }
        Ok(response.json().await?)
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, OrchestratorError> {
        let response = self
            .request(self.http.get(format!("{}{path}", self.base)))
            .timeout(Duration::from_secs(4))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(OrchestratorError::Status(response.status().as_u16()));
        }
        Ok(response.json().await?)
    }

    async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, OrchestratorError> {
        let response = self
            .request(self.http.post(format!("{}{path}", self.base)))
            .timeout(Duration::from_secs(4))
            .json(body)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(OrchestratorError::Status(response.status().as_u16()));
        }
        Ok(response.json().await?)
    }

    pub async fn friends(&self, player: Uuid) -> Result<FriendsList, OrchestratorError> {
        self.get(&format!("/api/v1/players/{player}/friends")).await
    }

    pub async fn friend(
        &self,
        action: &str,
        request: &FriendRequest,
    ) -> Result<FriendOutcome, OrchestratorError> {
        self.post(&format!("/api/v1/friends/{action}"), request)
            .await
    }

    pub async fn party_of(&self, player: Uuid) -> Result<Option<Party>, OrchestratorError> {
        self.get(&format!("/api/v1/players/{player}/party")).await
    }

    pub async fn party(
        &self,
        action: &str,
        request: &PartyRequest,
    ) -> Result<PartyOutcome, OrchestratorError> {
        self.post(&format!("/api/v1/parties/{action}"), request)
            .await
    }

    pub async fn ack(&self, transfer: Uuid, success: bool, message: Option<String>) {
        let body = TransferAckRequest { success, message };
        let sent = self
            .request(self.http.post(format!(
                "{}/api/v1/proxy/transfers/{transfer}/ack",
                self.base
            )))
            .timeout(Duration::from_secs(4))
            .json(&body)
            .send()
            .await;
        if let Err(error) = sent {
            tracing::warn!("Could not acknowledge transfer {transfer}: {error}");
        }
    }

    pub async fn disconnected(&self, player: Uuid) {
        let body = ProxyDisconnectRequest {
            player_uuid: player,
        };
        let sent = self
            .request(
                self.http
                    .post(format!("{}/api/v1/proxy/player-disconnected", self.base)),
            )
            .timeout(Duration::from_secs(4))
            .json(&body)
            .send()
            .await;
        if let Err(error) = sent {
            tracing::debug!("Could not report the disconnect of {player}: {error}");
        }
    }

    pub async fn stream_events(
        &self,
        events: &mpsc::UnboundedSender<ProxyEvent>,
    ) -> Result<(), OrchestratorError> {
        let response = self
            .request(self.http.get(format!("{}/api/v1/proxy/events", self.base)))
            .header("Accept", "text/event-stream")
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(OrchestratorError::Status(response.status().as_u16()));
        }
        let mut body = response.bytes_stream();
        let mut parser = SseParser::default();
        while let Some(chunk) = body.next().await {
            for payload in parser.feed(&chunk?) {
                match serde_json::from_str::<ProxyEvent>(&payload) {
                    Ok(event) => {
                        let _ = events.send(event);
                    }
                    Err(error) => tracing::debug!("Unknown proxy event {payload}: {error}"),
                }
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct SseParser {
    line: Vec<u8>,
    data: String,
}

impl SseParser {
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut complete = Vec::new();
        for byte in chunk {
            if *byte != b'\n' {
                self.line.push(*byte);
                continue;
            }
            let line = String::from_utf8_lossy(&self.line)
                .trim_end_matches('\r')
                .to_owned();
            self.line.clear();
            if line.is_empty() {
                if !self.data.is_empty() {
                    complete.push(std::mem::take(&mut self.data));
                }
            } else if let Some(part) = line.strip_prefix("data:") {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(part.trim_start());
            }
        }
        complete
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_split_across_chunks_come_out_whole() {
        let mut parser = SseParser::default();
        assert!(parser.feed(b"event: x\ndata: {\"type\":").is_empty());
        let events = parser.feed(b"\"server_remove\",\"name\":\"a\"}\n\ndata: second\r\n\r\n");
        assert_eq!(
            events,
            [
                "{\"type\":\"server_remove\",\"name\":\"a\"}".to_owned(),
                "second".to_owned()
            ]
        );
        let event: ProxyEvent = serde_json::from_str(&events[0]).unwrap();
        assert_eq!(
            event,
            ProxyEvent::ServerRemove {
                name: "a".to_owned()
            }
        );
    }

    #[test]
    fn comments_and_keepalives_are_ignored() {
        let mut parser = SseParser::default();
        assert!(parser.feed(b": keep-alive\n\n").is_empty());
    }
}
