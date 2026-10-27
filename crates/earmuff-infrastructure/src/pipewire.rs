//! The PipeWire sound system: finds a headset's sound card with `pw-dump`, makes it the
//! default with `wpctl` (WirePlumber, which remembers the choice), and records from its
//! mic and plays on its speakers with `pw-record` and `pw-play`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use earmuff_application::{AudioSystem, MicRecorder, Recording, Switch, meter};
use earmuff_domain::UsbId;
use serde_json::Value;

/// Talks to PipeWire through its command-line tools
#[derive(Clone, Debug)]
pub struct PipeWire {
    dump: PathBuf,
    wpctl: PathBuf,
    record: PathBuf,
    play: PathBuf,
}

impl Default for PipeWire {
    fn default() -> Self {
        PipeWire::with_programs("pw-dump", "wpctl").with_recorder("pw-record", "pw-play")
    }
}

/// The rate mic tests record at
const RATE: u32 = 48_000;
/// How much sound goes into one reading of the level meter
const METER_EVERY: Duration = Duration::from_millis(50);

impl PipeWire {
    /// Other programs in place of `pw-dump` and `wpctl`, e.g. scripted ones for tests
    pub fn with_programs(dump: impl AsRef<Path>, wpctl: impl AsRef<Path>) -> Self {
        let (dump, wpctl) = (dump.as_ref().to_path_buf(), wpctl.as_ref().to_path_buf());
        PipeWire { dump, wpctl, record: "pw-record".into(), play: "pw-play".into() }
    }

    /// Other programs in place of `pw-record` and `pw-play`
    pub fn with_recorder(mut self, record: impl AsRef<Path>, play: impl AsRef<Path>) -> Self {
        (self.record, self.play) = (record.as_ref().to_path_buf(), play.as_ref().to_path_buf());
        self
    }

    fn graph(&self, headset: UsbId) -> Result<Graph, String> {
        let output = Command::new(&self.dump).output().map_err(|error| format!("couldn't run pw-dump: {error}"))?;
        if !output.status.success() {
            return Err("pw-dump failed; is PipeWire running?".into());
        }
        parse(&String::from_utf8_lossy(&output.stdout), headset)
    }

    /// The raw-sound arguments pw-record and pw-play share
    fn raw_args(rate: u32, target: &str) -> Vec<String> {
        ["--raw", "--format", "s16", "--channels", "1", "--rate", &rate.to_string(), "--target", target]
            .map(String::from)
            .to_vec()
    }

    fn set_default(&self, node: &Node) -> Result<(), String> {
        let output = Command::new(&self.wpctl)
            .args(["set-default", &node.id.to_string()])
            .output()
            .map_err(|error| format!("couldn't run wpctl: {error}"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "wpctl couldn't switch to {}: {}",
                node.description,
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }
}

impl AudioSystem for PipeWire {
    fn use_headset(&self, headset: UsbId) -> Result<Switch, String> {
        let graph = self.graph(headset)?;
        let Some(sink) = &graph.sink else { return Ok(Switch::NotFound) };
        let mut switched = false;
        if graph.default_sink.as_deref() != Some(&sink.name) {
            self.set_default(sink)?;
            switched = true;
        }
        if let Some(source) = &graph.source
            && graph.default_source.as_deref() != Some(&source.name)
        {
            self.set_default(source)?;
            switched = true;
        }
        Ok(if switched { Switch::Switched(sink.description.clone()) } else { Switch::Already })
    }
}

impl MicRecorder for PipeWire {
    fn record(&self, headset: UsbId, duration: Duration, level: &mut dyn FnMut(f64)) -> Result<Recording, String> {
        let source = self.graph(headset)?.source.ok_or("the computer has no mic for this headset")?;
        let frames = (duration.as_secs_f64() * f64::from(RATE)).round() as u64;
        let mut child = Command::new(&self.record)
            .args(PipeWire::raw_args(RATE, &source.name))
            .args(["--media-category", "Capture", "--sample-count", &frames.to_string(), "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("couldn't run pw-record: {error}"))?;
        let mut stdout = child.stdout.take().ok_or("pw-record has no output")?;
        let chunk_bytes = (METER_EVERY.as_secs_f64() * f64::from(RATE)) as usize * 2;
        let (mut bytes, mut buffer) = (Vec::new(), vec![0; chunk_bytes]);
        loop {
            let read = stdout.read(&mut buffer).map_err(|error| format!("couldn't read the recording: {error}"))?;
            if read == 0 {
                break;
            }
            let start = bytes.len() - bytes.len() % 2;
            bytes.extend_from_slice(&buffer[..read]);
            level(meter(&samples(&bytes[start..])));
        }
        let output = child.wait_with_output().map_err(|error| format!("pw-record failed: {error}"))?;
        if !output.status.success() && bytes.is_empty() {
            return Err(format!("couldn't record: {}", String::from_utf8_lossy(&output.stderr).trim()));
        }
        Ok(Recording { rate: RATE, samples: samples(&bytes) })
    }

    fn play(&self, headset: UsbId, recording: &Recording) -> Result<(), String> {
        let sink = self.graph(headset)?.sink.ok_or("the computer has no speakers for this headset")?;
        let mut child = Command::new(&self.play)
            .args(PipeWire::raw_args(recording.rate, &sink.name))
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("couldn't run pw-play: {error}"))?;
        let bytes: Vec<u8> = recording.samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let written = child.stdin.take().ok_or("pw-play takes no input")?.write_all(&bytes);
        let output = child.wait_with_output().map_err(|error| format!("pw-play failed: {error}"))?;
        match (written, output.status.success()) {
            (Ok(()), true) => Ok(()),
            _ => Err(format!("couldn't play it back: {}", String::from_utf8_lossy(&output.stderr).trim())),
        }
    }
}

/// Little-endian 16-bit samples; a byte left over is dropped
fn samples(bytes: &[u8]) -> Vec<i16> {
    bytes.chunks_exact(2).map(|pair| i16::from_le_bytes([pair[0], pair[1]])).collect()
}

/// A sink (speakers) or source (mic)
#[derive(Clone, Debug, PartialEq, Eq)]
struct Node {
    id: u64,
    name: String,
    description: String,
    priority: i64,
}

/// What PipeWire knows about one headset
#[derive(Debug, Default, PartialEq, Eq)]
struct Graph {
    /// The headset's main sink and source, if it has them
    sink: Option<Node>,
    source: Option<Node>,
    /// The names of the current default sink and source
    default_sink: Option<String>,
    default_source: Option<String>,
}

fn hex_id(props: &Value, key: &str) -> Option<u16> {
    u16::from_str_radix(props.get(key)?.as_str()?.trim_start_matches("0x"), 16).ok()
}

/// A metadata value: an object like {"name": "…"}, or the same as a JSON string
fn default_name(value: &Value) -> Option<String> {
    let parsed;
    let object = match value {
        Value::String(text) => {
            parsed = serde_json::from_str::<Value>(text).ok()?;
            &parsed
        }
        other => other,
    };
    Some(object.get("name")?.as_str()?.to_string())
}

/// Reads `pw-dump`'s JSON: the headset's nodes, and the current defaults
fn parse(dump: &str, headset: UsbId) -> Result<Graph, String> {
    let objects: Vec<Value> =
        serde_json::from_str(dump).map_err(|error| format!("pw-dump printed something odd: {error}"))?;
    let props = |object: &Value| object.get("info").and_then(|info| info.get("props")).cloned().unwrap_or_default();
    let kind = |object: &Value| object.get("type").and_then(Value::as_str).unwrap_or_default().to_string();

    let devices: Vec<u64> = objects
        .iter()
        .filter(|object| kind(object) == "PipeWire:Interface:Device")
        .filter(|object| {
            let props = props(object);
            hex_id(&props, "device.vendor.id") == Some(headset.vendor)
                && hex_id(&props, "device.product.id") == Some(headset.product)
        })
        .filter_map(|object| object.get("id")?.as_u64())
        .collect();

    let mut graph = Graph::default();
    for object in &objects {
        match kind(object).as_str() {
            "PipeWire:Interface:Node" => {
                let props = props(object);
                let Some(device) = props.get("device.id").and_then(Value::as_u64) else { continue };
                if !devices.contains(&device) {
                    continue;
                }
                let text = |key: &str| props.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
                let Some(id) = object.get("id").and_then(Value::as_u64) else { continue };
                let node = Node {
                    id,
                    name: text("node.name"),
                    description: text("node.description"),
                    priority: props.get("priority.session").and_then(Value::as_i64).unwrap_or_default(),
                };
                let slot = match props.get("media.class").and_then(Value::as_str) {
                    Some("Audio/Sink") => &mut graph.sink,
                    Some("Audio/Source") => &mut graph.source,
                    _ => continue,
                };
                // a headset with several (e.g. game and chat) uses the one PipeWire ranks first
                if slot.as_ref().is_none_or(|other| node.priority > other.priority) {
                    *slot = Some(node);
                }
            }
            "PipeWire:Interface:Metadata" => {
                let name = object.get("props").and_then(|p| p.get("metadata.name")).and_then(Value::as_str);
                if name != Some("default") {
                    continue;
                }
                for entry in object.get("metadata").and_then(Value::as_array).into_iter().flatten() {
                    let value = entry.get("value").and_then(default_name);
                    match entry.get("key").and_then(Value::as_str) {
                        Some("default.audio.sink") => graph.default_sink = value,
                        Some("default.audio.source") => graph.default_source = value,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = include_str!("../tests/data/pw-dump.json");
    const CLOUD_ALPHA: UsbId = UsbId::new(0x03F0, 0x098D);

    #[test]
    fn finds_the_headsets_speakers_and_mic() {
        let graph = parse(DUMP, CLOUD_ALPHA).unwrap();
        let sink = graph.sink.unwrap();
        assert_eq!((sink.id, sink.description.as_str()), (57, "HyperX Cloud Alpha Wireless Analog Stereo"));
        assert_eq!(graph.source.unwrap().id, 58);
        assert_eq!(graph.default_sink.as_deref(), Some("alsa_output.pci-0000_00_1f.3.iec958-stereo"));
        assert_eq!(graph.default_source.as_deref(), Some("alsa_input.pci-0000_00_1f.3.analog-stereo"));
    }

    #[test]
    fn a_headset_it_doesnt_know_has_no_nodes() {
        let graph = parse(DUMP, UsbId::new(0x1234, 0x5678)).unwrap();
        assert_eq!((graph.sink, graph.source), (None, None));
    }

    #[test]
    fn picks_the_sink_pipewire_ranks_first() {
        let dump = r#"[
            {"id": 1, "type": "PipeWire:Interface:Device",
             "info": {"props": {"device.vendor.id": "0x1038", "device.product.id": "0x12e0"}}},
            {"id": 2, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Audio/Sink",
             "node.name": "chat", "node.description": "Chat", "device.id": 1, "priority.session": 100}}},
            {"id": 3, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Audio/Sink",
             "node.name": "game", "node.description": "Game", "device.id": 1, "priority.session": 200}}},
            {"id": 4, "type": "PipeWire:Interface:Metadata", "props": {"metadata.name": "default"},
             "metadata": [{"subject": 0, "key": "default.audio.sink", "value": "{\"name\": \"game\"}"}]}
        ]"#;
        let graph = parse(dump, UsbId::new(0x1038, 0x12E0)).unwrap();
        assert_eq!(graph.sink.map(|n| n.name), Some("game".into()));
        assert_eq!(graph.default_sink.as_deref(), Some("game"));
        assert_eq!(graph.source, None);
    }

    #[test]
    fn samples_are_little_endian() {
        assert_eq!(samples(&[0x01, 0x00, 0xFF, 0xFF, 0x00, 0x80, 0x07]), [1, -1, i16::MIN]);
    }

    #[test]
    fn says_so_when_the_dump_is_not_json() {
        assert!(parse("not json", CLOUD_ALPHA).is_err());
    }
}
