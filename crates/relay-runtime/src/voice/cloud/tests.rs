use super::*;
use futures_util::future;
use relay_core::voice::SpeechAudioStream;
use serde_json::{Value, json};
use std::{
    net::{TcpListener, TcpStream},
    sync::{Mutex, atomic::Ordering},
    thread,
    time::Instant,
};
use tokio_tungstenite::tungstenite::{self, Message, WebSocket};

// Tungstenite fixes the handshake callback's error to an unboxed HTTP response.
#[allow(clippy::result_large_err)]
fn server(
    f: impl FnOnce(WebSocket<TcpStream>) + Send + 'static,
) -> (config::Config, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let socket = tungstenite::accept_hdr(
            stream,
            |request: &tungstenite::handshake::server::Request, response| {
                assert_eq!(request.headers()["Authorization"], "Bearer unit-test-key");
                Ok(response)
            },
        )
        .unwrap();
        f(socket);
    });
    (
        config::Config {
            endpoint: format!("ws://{address}"),
            key: "unit-test-key".into(),
            workspace: None,
        },
        worker,
    )
}
fn receive(socket: &mut WebSocket<TcpStream>) -> Value {
    serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap()
}
fn send(socket: &mut WebSocket<TcpStream>, id: &str, name: &str, payload: Value) {
    socket
        .send(transport::message(
            json!({"header":{"event":name,"task_id":id},"payload":payload}),
        ))
        .unwrap();
}
fn wav(samples: &[i16]) -> Vec<u8> {
    let n = samples.len() as u32 * 2;
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((n + 36).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16_u32.to_le_bytes());
    bytes.extend([1, 0, 1, 0]);
    bytes.extend(16_000_u32.to_le_bytes());
    bytes.extend(32_000_u32.to_le_bytes());
    bytes.extend([2, 0, 16, 0]);
    bytes.extend(b"data");
    bytes.extend(n.to_le_bytes());
    for s in samples {
        bytes.extend(s.to_le_bytes());
    }
    bytes
}

#[test]
fn asr_uploads_only_after_ack_and_collects_final_sentences_without_duplicates() {
    let (config, server) = server(|mut socket| {
        let request = receive(&mut socket);
        let id = request["header"]["task_id"].as_str().unwrap();
        assert!(uuid::Uuid::parse_str(id).is_ok());
        assert_eq!(request["payload"]["model"], asr::MODEL);
        assert_eq!(request["payload"]["parameters"]["sample_rate"], 16_000);
        assert_eq!(
            request["payload"]["parameters"]["disfluency_removal_enabled"],
            false
        );
        send(&mut socket, id, "task-started", json!({}));
        let mut bytes = 0;
        loop {
            match socket.read().unwrap() {
                Message::Binary(data) => bytes += data.len(),
                Message::Text(text) => {
                    let request: Value = serde_json::from_str(&text).unwrap();
                    assert_eq!(request["header"]["task_id"], id);
                    assert_eq!(request["header"]["action"], "finish-task");
                    break;
                }
                _ => panic!("Unexpected upload frame"),
            }
        }
        assert_eq!(bytes, 40_000);
        for (sentence_id, text, sentence_end, heartbeat) in [
            (1, "partial", false, false),
            (0, "", true, true),
            (2, "世界", true, false),
            (1, "你好", true, false),
            (1, "您好", true, false),
        ] {
            send(
                &mut socket,
                id,
                "result-generated",
                json!({"output":{"sentence":{ "sentence_id":sentence_id,"text":text,"sentence_end":sentence_end,"heartbeat":heartbeat}}}),
            );
        }
        send(&mut socket, id, "task-finished", json!({}));
    });
    let audio = wav(&vec![100; 20_000]);
    let text = transport::execute(
        &AtomicBool::new(false),
        Duration::from_secs(3),
        asr::transcribe(&config, &audio),
    )
    .unwrap();
    assert_eq!(text, "您好 世界");
    server.join().unwrap();
}

#[derive(Default)]
struct Output {
    samples: Arc<Mutex<Vec<i16>>>,
    stopped: Arc<AtomicBool>,
}
struct Stream {
    samples: Arc<Mutex<Vec<i16>>>,
    stopped: Arc<AtomicBool>,
    blocked: usize,
}
impl Drop for Stream {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
    }
}
impl SpeechAudioOutput for Output {
    fn open(&self, sample_rate: u32) -> Result<Box<dyn SpeechAudioStream>, String> {
        assert_eq!(sample_rate, 24_000);
        Ok(Box::new(Stream {
            samples: self.samples.clone(),
            stopped: self.stopped.clone(),
            blocked: 2,
        }))
    }
}
impl SpeechAudioStream for Stream {
    fn try_write(&mut self, samples: &[i16]) -> Result<bool, String> {
        if self.blocked > 0 {
            self.blocked -= 1;
            return Ok(false);
        }
        self.samples.lock().unwrap().extend(samples);
        Ok(true)
    }
    fn is_drained(&self) -> Result<bool, String> {
        Ok(true)
    }
}
#[test]
fn tts_streams_pcm_across_frame_boundaries_and_honors_playback_backpressure() {
    let (config, server) = server(|mut socket| {
        let request = receive(&mut socket);
        let id = request["header"]["task_id"].as_str().unwrap();
        assert_eq!(request["payload"]["model"], tts::MODEL);
        assert_eq!(request["payload"]["parameters"]["voice"], "longanhuan_v3.1");
        assert_eq!(request["payload"]["parameters"]["format"], "pcm");
        send(&mut socket, id, "task-started", json!({}));
        let request = receive(&mut socket);
        assert_eq!(request["header"]["task_id"], id);
        assert_eq!(request["payload"]["input"]["text"], "你好");
        assert_eq!(receive(&mut socket)["header"]["action"], "finish-task");
        for bytes in [vec![0xff], vec![0x7f, 0], vec![0x80, 0, 0]] {
            socket.send(Message::Binary(bytes.into())).unwrap();
        }
        send(&mut socket, id, "task-finished", json!({}));
    });
    let output = Output::default();
    transport::execute(
        &AtomicBool::new(false),
        Duration::from_secs(3),
        tts::speak(&config, "你好", &output),
    )
    .unwrap();
    assert_eq!(*output.samples.lock().unwrap(), [i16::MAX, i16::MIN, 0]);
    assert!(output.stopped.load(Ordering::Acquire));
    server.join().unwrap();
}

#[test]
fn cancelled_network_wait_drops_its_work_promptly() {
    let cancelled = AtomicBool::new(false);
    let start = Instant::now();
    thread::scope(|scope| {
        scope.spawn(|| {
            thread::sleep(Duration::from_millis(40));
            cancelled.store(true, Ordering::Release);
        });
        let result: Result<(), String> =
            transport::execute(&cancelled, Duration::from_secs(10), future::pending());
        assert!(result.unwrap_err().contains("cancelled"));
    });
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn invalid_audio_and_unrelated_or_failed_tasks_never_produce_a_transcript() {
    assert!(asr::pcm(b"not audio").is_err());
    let mut audio = wav(&[1, 2]);
    audio[24..28].copy_from_slice(&44_100_u32.to_le_bytes());
    assert!(asr::pcm(&audio).is_err());
    let error = transport::event(&json!({"header":{"task_id":"correct","event":"task-failed","error_code":"InvalidApiKey","error_message":"unit-test-key"}}).to_string(),"correct").unwrap_err();
    assert!(error.contains("authentication"));
    assert!(!error.contains("unit-test-key"));
    assert!(
        transport::event(
            &json!({"header":{"task_id":"other","event":"task-finished"}}).to_string(),
            "correct"
        )
        .is_err()
    );
    let mut decoder = tts::PcmDecoder::default();
    assert!(decoder.finish().is_err());
    decoder.decode(&[1]).unwrap();
    assert!(decoder.finish().is_err());
}

#[test]
fn asr_disconnect_discards_even_final_sentences_until_task_finished() {
    let (config, server) = server(|mut socket| {
        let request = receive(&mut socket);
        let id = request["header"]["task_id"].as_str().unwrap();
        send(&mut socket, id, "task-started", json!({}));
        socket.read().unwrap();
        receive(&mut socket);
        send(
            &mut socket,
            id,
            "result-generated",
            json!({"output":{"sentence":{"sentence_id":1,"sentence_end":true,"text":"Do not dispatch"}}}),
        );
        socket.close(None).unwrap();
    });
    let audio = wav(&[1, 2]);
    assert!(
        transport::execute(
            &AtomicBool::new(false),
            Duration::from_secs(3),
            asr::transcribe(&config, &audio)
        )
        .is_err()
    );
    server.join().unwrap();
}
