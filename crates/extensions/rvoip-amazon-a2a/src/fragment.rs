use crate::{
    protocol::{self, Request, EVENT_TYPE},
    A2aError, AmazonA2aConfig, Result,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::time::Instant;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fragment {
    fragment_id: String,
    part_number: usize,
    total_parts: usize,
    payload: String,
    encoding: String,
}
struct Assembly {
    id: String,
    parts: Vec<Option<String>>,
    bytes: usize,
    deadline: Instant,
}
/// One bounded assembly per socket. A second interleaved message fails closed.
#[derive(Default)]
pub(crate) struct Reassembler {
    pending: Option<Assembly>,
}
impl Reassembler {
    pub fn deadline(&self) -> Option<Instant> {
        self.pending.as_ref().map(|a| a.deadline)
    }
    pub fn push(&mut self, text: &str, config: &AmazonA2aConfig) -> Result<Option<Request>> {
        if text.len() > config.max_frame_bytes {
            return Err(A2aError::Capacity);
        }
        let request = Request::parse(text)?;
        if request.event() != Some("FRAGMENT") {
            return Ok(Some(request));
        }
        let f: Fragment = request.data("fragment")?;
        if f.encoding != "base64"
            || f.fragment_id.is_empty()
            || f.fragment_id.len() > 512
            || f.total_parts == 0
            || f.total_parts > config.max_fragment_parts
            || f.part_number >= f.total_parts
        {
            return Err(A2aError::Protocol);
        }
        let a = self.pending.get_or_insert_with(|| Assembly {
            id: f.fragment_id.clone(),
            parts: vec![None; f.total_parts],
            bytes: 0,
            deadline: Instant::now() + config.fragment_timeout,
        });
        if a.deadline <= Instant::now() {
            return Err(A2aError::Timeout);
        }
        if a.id != f.fragment_id
            || a.parts.len() != f.total_parts
            || a.parts[f.part_number].is_some()
        {
            return Err(A2aError::Protocol);
        }
        a.bytes = a
            .bytes
            .checked_add(f.payload.len())
            .ok_or(A2aError::Capacity)?;
        if a.bytes > config.max_payload_bytes.div_ceil(3) * 4 {
            return Err(A2aError::Capacity);
        }
        a.parts[f.part_number] = Some(f.payload);
        if a.parts.iter().any(Option::is_none) {
            return Ok(None);
        }
        let encoded = a
            .parts
            .iter()
            .filter_map(Option::as_deref)
            .collect::<String>();
        self.pending = None;
        let decoded = STANDARD.decode(encoded).map_err(|_| A2aError::Protocol)?;
        if decoded.len() > config.max_payload_bytes {
            return Err(A2aError::Capacity);
        }
        let request =
            Request::parse(std::str::from_utf8(&decoded).map_err(|_| A2aError::Protocol)?)?;
        if request.event() == Some("FRAGMENT")
            || request.params.message.parts.iter().any(|p| p.raw.is_some())
        {
            return Err(A2aError::Protocol);
        }
        Ok(Some(request))
    }
}

pub(crate) fn encode(value: Value, config: &AmazonA2aConfig) -> Result<Vec<String>> {
    let text = serde_json::to_string(&value).map_err(|_| A2aError::Protocol)?;
    if text.len() <= config.max_frame_bytes {
        return Ok(vec![text]);
    }
    if text.len() > config.max_payload_bytes {
        return Err(A2aError::Capacity);
    }
    let encoded = STANDARD.encode(text);
    let chunk_size = config.max_frame_bytes - 1024;
    let total = encoded.len().div_ceil(chunk_size);
    if total > config.max_fragment_parts {
        return Err(A2aError::Capacity);
    }
    let fragment_id = protocol::id();
    encoded.as_bytes().chunks(chunk_size).enumerate().map(|(i, chunk)| {
        let payload = std::str::from_utf8(chunk).map_err(|_| A2aError::Protocol)?;
        let frame = protocol::envelope(json!({"message":{"messageId":protocol::id(),"role":"ROLE_AGENT","parts":[{"data":{"fragment":{"fragmentId":fragment_id,"partNumber":i,"totalParts":total,"payload":payload,"encoding":"base64"}},"mediaType":"application/json"}],"metadata":{EVENT_TYPE:"FRAGMENT"}}}));
        let frame = frame.to_string();
        if frame.len() > config.max_frame_bytes { return Err(A2aError::Capacity); }
        Ok(frame)
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ApiKey;
    fn config() -> AmazonA2aConfig {
        let mut c = AmazonA2aConfig::new(ApiKey::new("test-key-for-fragments").unwrap());
        c.max_frame_bytes = 2048;
        c
    }
    fn frame(id: &str, n: usize, total: usize, payload: &str) -> String {
        json!({"jsonrpc":"2.0","id":"fragment-rpc","method":"SendMessage","params":{"message":{"messageId":"fragment","parts":[{"data":{"fragment":{"fragmentId":id,"partNumber":n,"totalParts":total,"payload":payload,"encoding":"base64"}}}],"metadata":{EVENT_TYPE:"FRAGMENT"}}}}).to_string()
    }
    fn payload() -> String {
        json!({"jsonrpc":"2.0","id":42,"method":"SendMessage","params":{"message":{"messageId":"text","contextId":"context","parts":[{"text":"é𝄞 hello"}]}}}).to_string()
    }
    #[test]
    fn reverse_order_reassembles_before_decoding_even_across_base64_quartets() {
        let cfg = config();
        let encoded = STANDARD.encode(payload());
        let mut r = Reassembler::default();
        assert!(r
            .push(&frame("f", 1, 2, &encoded[7..]), &cfg)
            .unwrap()
            .is_none());
        let request = r
            .push(&frame("f", 0, 2, &encoded[..7]), &cfg)
            .unwrap()
            .unwrap();
        assert_eq!(request.id, 42);
        assert!(r.pending.is_none());
    }
    #[test]
    fn rejects_duplicate_conflicting_and_excessive_parts() {
        let cfg = config();
        for second in [
            frame("f", 0, 2, "x"),
            frame("other", 1, 2, "x"),
            frame("f", 1, 3, "x"),
        ] {
            let mut r = Reassembler::default();
            r.push(&frame("f", 0, 2, "e"), &cfg).unwrap();
            assert!(r.push(&second, &cfg).is_err());
        }
        assert!(Reassembler::default()
            .push(&frame("f", 0, cfg.max_fragment_parts + 1, "x"), &cfg)
            .is_err());
    }
    #[tokio::test(start_paused = true)]
    async fn fragments_expire_without_allocating_new_assemblies() {
        let cfg = config();
        let mut r = Reassembler::default();
        r.push(&frame("f", 0, 2, "e"), &cfg).unwrap();
        tokio::time::advance(cfg.fragment_timeout).await;
        assert!(matches!(
            r.push(&frame("f", 1, 2, "A=="), &cfg),
            Err(A2aError::Timeout)
        ));
    }
    #[test]
    fn output_fragments_fit_budget_and_roundtrip() {
        let cfg = config();
        let value = json!({"unicode":"𝄞".repeat(2000)});
        let frames = encode(value.clone(), &cfg).unwrap();
        assert!(frames.len() > 1);
        let mut encoded = String::new();
        for frame in frames {
            assert!(frame.len() <= cfg.max_frame_bytes);
            let v: Value = serde_json::from_str(&frame).unwrap();
            encoded.push_str(
                v["result"]["message"]["parts"][0]["data"]["fragment"]["payload"]
                    .as_str()
                    .unwrap(),
            );
        }
        assert_eq!(
            serde_json::from_slice::<Value>(&STANDARD.decode(encoded).unwrap()).unwrap(),
            value
        );
    }
    #[test]
    fn nested_and_audio_fragments_are_rejected() {
        let cfg = config();
        let raw=json!({"jsonrpc":"2.0","id":1,"method":"SendMessage","params":{"message":{"messageId":"audio","contextId":"c","parts":[{"raw":"AAA=","mediaType":"audio/lpcm"}]}}}).to_string();
        for original in [frame("nested", 0, 1, "eA=="), raw] {
            assert!(Reassembler::default()
                .push(&frame("f", 0, 1, &STANDARD.encode(original)), &cfg)
                .is_err());
        }
    }
}
