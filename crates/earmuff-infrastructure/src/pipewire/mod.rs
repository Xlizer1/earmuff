//! The PipeWire sound system: finds a headset's sound card with `pw-dump`, makes it the
//! default with `wpctl` (WirePlumber, which remembers the choice), and records from its
//! mic and plays on its speakers with `pw-record` and `pw-play`. [`equalizer`] runs
//! software equalizers in front of its speakers.

pub mod equalizer;

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

    pub(crate) fn graph(&self, headset: UsbId) -> Result<Graph, String> {
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

    pub(crate) fn set_default(&self, node: &Node) -> Result<(), String> {
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
        // with Earmuff's equalizer in front of the headset, sound goes through that
        let Some(sink) = graph.equalizer.as_ref().or(graph.sink.as_ref()) else { return Ok(Switch::NotFound) };
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
pub(crate) struct Node {
    pub id: u64,
    pub name: String,
    pub description: String,
    priority: i64,
}

/// What PipeWire knows about one headset
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Graph {
    /// The headset's main sink and source, if it has them
    pub sink: Option<Node>,
    pub source: Option<Node>,
    /// Earmuff's equalizer in front of the headset's speakers, and the process running it
    pub equalizer: Option<Node>,
    pub equalizer_process: Option<i32>,
    /// The names of the current default sink and source
    pub default_sink: Option<String>,
    pub default_source: Option<String>,
}

/// What Earmuff calls the sink of a headset's equalizer
pub(crate) fn equalizer_name(headset: UsbId) -> String {
    format!("earmuff_eq_{:04x}_{:04x}", headset.vendor, headset.product)
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

    let equalizer = equalizer_name(headset);
    let (mut graph, mut equalizer_client, mut processes) = (Graph::default(), None, Vec::new());
    for object in &objects {
        match kind(object).as_str() {
            "PipeWire:Interface:Node" => {
                let props = props(object);
                let text = |key: &str| props.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
                let Some(id) = object.get("id").and_then(Value::as_u64) else { continue };
                let node = Node {
                    id,
                    name: text("node.name"),
                    description: text("node.description"),
                    priority: props.get("priority.session").and_then(Value::as_i64).unwrap_or_default(),
                };
                if node.name == equalizer {
                    equalizer_client = props.get("client.id").and_then(Value::as_u64);
                    graph.equalizer = Some(node);
                    continue;
                }
                let Some(device) = props.get("device.id").and_then(Value::as_u64) else { continue };
                if !devices.contains(&device) {
                    continue;
                }
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
            "PipeWire:Interface:Client" => {
                let props = props(object);
                let pid = props.get("application.process.id").and_then(|pid| match pid {
                    Value::String(text) => text.parse().ok(),
                    number => number.as_i64().and_then(|n| i32::try_from(n).ok()),
                });
                if let (Some(id), Some(pid)) = (object.get("id").and_then(Value::as_u64), pid) {
                    processes.push((id, pid));
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
    graph.equalizer_process =
        equalizer_client.and_then(|client| processes.iter().find(|(id, _)| *id == client).map(|(_, pid)| *pid));
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = include_str!("../../tests/data/pw-dump.json");
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
    fn finds_earmuffs_equalizer_and_its_process() {
        let mut objects: Vec<Value> = serde_json::from_str(DUMP).unwrap();
        objects.push(serde_json::json!({"id": 200, "type": "PipeWire:Interface:Node", "info": {"props": {
            "media.class": "Audio/Sink", "node.name": "earmuff_eq_03f0_098d", "client.id": 300}}}));
        objects.push(serde_json::json!({"id": 300, "type": "PipeWire:Interface:Client",
            "info": {"props": {"application.process.id": 4321}}}));
        let graph = parse(&serde_json::to_string(&objects).unwrap(), CLOUD_ALPHA).unwrap();
        assert_eq!(graph.equalizer.map(|node| node.id), Some(200));
        assert_eq!(graph.equalizer_process, Some(4321));
        // another headset's equalizer isn't this one's
        let other = parse(&serde_json::to_string(&objects).unwrap(), UsbId::new(1, 2)).unwrap();
        assert_eq!(other.equalizer, None);
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
