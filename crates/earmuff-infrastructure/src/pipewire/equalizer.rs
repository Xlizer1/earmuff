//! The software equalizer: a PipeWire filter chain of ten peaking filters in front of a
//! headset's speakers, run as its own `pipewire -c` process. It shows up as a sink of its
//! own; while it's the computer's output, everything played goes through it to the headset.
//! Moving a band re-tunes it in place with `pw-cli`.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use earmuff_application::{SOFTWARE_EQ_FREQUENCIES, SoftwareEqualizer};
use earmuff_domain::{Gain, UsbId};
use rustix::process::{Pid, Signal, kill_process};

use super::{Node, PipeWire, equalizer_name};

/// How wide each band is: an octave
const Q: f64 = 1.41;
/// How long PipeWire gets to set an equalizer up, and to take one down
const START_WAIT: Duration = Duration::from_secs(3);
const STOP_WAIT: Duration = Duration::from_secs(1);

pub struct PipeWireEqualizer {
    pipewire: PipeWire,
    daemon: PathBuf,
    cli: PathBuf,
    /// Where the filter chains' configuration goes
    dir: PathBuf,
    running: Mutex<HashMap<UsbId, Child>>,
}

impl PipeWireEqualizer {
    /// Keeps its configuration in `$XDG_RUNTIME_DIR/earmuff`
    pub fn new(pipewire: PipeWire) -> Self {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
        PipeWireEqualizer::with_programs(pipewire, "pipewire", "pw-cli", &runtime.join("earmuff"))
    }

    /// Other programs in place of `pipewire` and `pw-cli`, e.g. scripted ones for tests
    pub fn with_programs(pipewire: PipeWire, daemon: impl AsRef<Path>, cli: impl AsRef<Path>, dir: &Path) -> Self {
        PipeWireEqualizer {
            pipewire,
            daemon: daemon.as_ref().to_path_buf(),
            cli: cli.as_ref().to_path_buf(),
            dir: dir.to_path_buf(),
            running: Mutex::default(),
        }
    }

    fn start(&self, headset: UsbId, name: &str, sink: &Node, curve: &[Gain]) -> Result<Child, String> {
        let path = self.dir.join(format!("eq-{:04x}-{:04x}.conf", headset.vendor, headset.product));
        std::fs::create_dir_all(&self.dir)
            .and_then(|()| std::fs::write(&path, config(headset, name, &sink.name, curve)))
            .map_err(|error| format!("couldn't write the equalizer's settings: {error}"))?;
        Command::new(&self.daemon)
            .arg("-c")
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("couldn't run pipewire: {error}"))
    }

    /// Waits for a freshly started equalizer's sink to show up
    fn wait_for(&self, headset: UsbId, child: &mut Child) -> Result<Node, String> {
        let end = Instant::now() + START_WAIT;
        let pid = i32::try_from(child.id()).ok();
        loop {
            let graph = self.pipewire.graph(headset)?;
            if let Some(node) = graph.equalizer
                && graph.equalizer_process == pid
            {
                return Ok(node);
            }
            if !matches!(child.try_wait(), Ok(None)) || Instant::now() >= end {
                end_process(child);
                return Err("PipeWire couldn't set the equalizer up".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn tune(&self, node: &Node, curve: &[Gain]) -> Result<(), String> {
        let output = Command::new(&self.cli)
            .args(["set-param", &node.id.to_string(), "Props", &params(curve)])
            .output()
            .map_err(|error| format!("couldn't run pw-cli: {error}"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!("couldn't change the equalizer: {}", String::from_utf8_lossy(&output.stderr).trim()))
        }
    }

    fn running(&self) -> std::sync::MutexGuard<'_, HashMap<UsbId, Child>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl SoftwareEqualizer for PipeWireEqualizer {
    fn apply(&self, headset: UsbId, name: &str, curve: &[Gain]) -> Result<bool, String> {
        let mut running = self.running();
        let graph = self.pipewire.graph(headset)?;
        let alive = running.get_mut(&headset).is_some_and(|child| matches!(child.try_wait(), Ok(None)));
        if let (true, Some(node)) = (alive, &graph.equalizer) {
            return self.tune(node, curve).map(|()| true);
        }
        if let Some(mut child) = running.remove(&headset) {
            end_process(&mut child);
        } else if let Some(pid) = graph.equalizer_process {
            // left over from an Earmuff that didn't get to quit properly
            terminate(pid);
        }
        let Some(sink) = &graph.sink else { return Ok(false) };
        let mut child = self.start(headset, name, sink, curve)?;
        let node = self.wait_for(headset, &mut child)?;
        running.insert(headset, child);
        // sound that went to the headset now goes through its equalizer
        if graph.default_sink.as_deref() == Some(&sink.name) {
            self.pipewire.set_default(&node)?;
        }
        Ok(true)
    }

    fn stop(&self, headset: UsbId) -> Result<(), String> {
        let child = self.running().remove(&headset);
        let graph = self.pipewire.graph(headset)?;
        match child {
            Some(mut child) => end_process(&mut child),
            None => graph.equalizer_process.into_iter().for_each(terminate),
        }
        // sound that went through the equalizer goes straight to the headset again
        if let (Some(equalizer), Some(sink)) = (&graph.equalizer, &graph.sink)
            && graph.default_sink.as_deref() == Some(&equalizer.name)
        {
            self.pipewire.set_default(sink)?;
        }
        Ok(())
    }
}

impl Drop for PipeWireEqualizer {
    fn drop(&mut self) {
        for (_, mut child) in self.running().drain() {
            end_process(&mut child);
        }
    }
}

fn terminate(pid: i32) {
    if let Some(pid) = Pid::from_raw(pid) {
        let _ = kill_process(pid, Signal::TERM);
    }
}

/// Asks the process to finish, and makes it if it doesn't
fn end_process(child: &mut Child) {
    let _ = kill_process(Pid::from_child(child), Signal::TERM);
    let end = Instant::now() + STOP_WAIT;
    while Instant::now() < end {
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// A string for PipeWire's configuration language
fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Turns the volume down ahead of the bands by as much as the biggest boost, so a boosted
/// curve can't clip
fn preamp(curve: &[Gain]) -> f64 {
    let boost = curve.iter().map(|gain| gain.db()).fold(0.0, f64::max);
    10f64.powf(-boost / 20.0)
}

/// Every band's gain and the preamp, for `pw-cli set-param … Props`
fn params(curve: &[Gain]) -> String {
    let mut params = String::from("{ params = [");
    for (band, gain) in curve.iter().enumerate() {
        let _ = write!(params, " \"eq_band_{}:Gain\" {:.1}", band + 1, gain.db());
    }
    let _ = write!(params, " \"preamp:Gain 1\" {:.4} ] }}", preamp(curve));
    params
}

/// The configuration of a `pipewire -c` process that runs one headset's equalizer
fn config(headset: UsbId, name: &str, sink: &str, curve: &[Gain]) -> String {
    let mut nodes = format!(
        "          {{ type = builtin name = preamp label = mixer control = {{ \"Gain 1\" = {:.4} }} }}\n",
        preamp(curve)
    );
    let mut links = String::from("          { output = \"preamp:Out\" input = \"eq_band_1:In\" }\n");
    for (band, frequency) in SOFTWARE_EQ_FREQUENCIES.iter().enumerate() {
        let gain = curve.get(band).map_or(0.0, |gain| gain.db());
        let _ = writeln!(
            nodes,
            "          {{ type = builtin name = eq_band_{n} label = bq_peaking \
             control = {{ \"Freq\" = {frequency}.0 \"Q\" = {Q} \"Gain\" = {gain:.1} }} }}",
            n = band + 1,
        );
        if band > 0 {
            let _ =
                writeln!(links, "          {{ output = \"eq_band_{band}:Out\" input = \"eq_band_{}:In\" }}", band + 1);
        }
    }
    let node = equalizer_name(headset);
    let description = quoted(&format!("{name} (Earmuff equalizer)"));
    format!(
        r#"# Written by Earmuff for its equalizer; changes here are overwritten
context.properties = {{ log.level = 0 }}
context.spa-libs = {{
  audio.convert.* = audioconvert/libspa-audioconvert
  support.* = support/libspa-support
}}
context.modules = [
  {{ name = libpipewire-module-rt args = {{ }} flags = [ ifexists nofail ] }}
  {{ name = libpipewire-module-protocol-native }}
  {{ name = libpipewire-module-client-node }}
  {{ name = libpipewire-module-adapter }}
  {{ name = libpipewire-module-filter-chain
    args = {{
      node.description = {description}
      media.name = {description}
      filter.graph = {{
        nodes = [
{nodes}        ]
        links = [
{links}        ]
        inputs = [ "preamp:In 1" ]
        outputs = [ "eq_band_{bands}:Out" ]
      }}
      audio.channels = 2
      audio.position = [ FL FR ]
      capture.props = {{
        node.name = {capture}
        media.class = Audio/Sink
      }}
      playback.props = {{
        node.name = {playback}
        node.passive = true
        target.object = {target}
      }}
    }}
  }}
]
"#,
        bands = SOFTWARE_EQ_FREQUENCIES.len(),
        capture = quoted(&node),
        playback = quoted(&format!("{node}.output")),
        target = quoted(sink),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bass() -> Vec<Gain> {
        [60, 50, 40, 20, 0, 0, 0, 0, 0, -25].map(Gain::tenths).to_vec()
    }

    #[test]
    fn the_preamp_makes_room_for_the_biggest_boost() {
        assert_eq!(preamp(&[Gain::tenths(0), Gain::tenths(-30)]), 1.0);
        assert!((preamp(&bass()) - 0.501).abs() < 0.001, "6 dB down: {}", preamp(&bass()));
    }

    #[test]
    fn params_set_every_band_and_the_preamp() {
        let params = params(&bass());
        assert!(params.starts_with("{ params = [ \"eq_band_1:Gain\" 6.0 \"eq_band_2:Gain\" 5.0"), "{params}");
        assert!(params.contains("\"eq_band_10:Gain\" -2.5 \"preamp:Gain 1\" 0.5012 ] }"), "{params}");
    }

    #[test]
    fn the_config_chains_ten_bands_in_front_of_the_headset() {
        let config = config(UsbId::new(0x03F0, 0x098D), "Cloud \"Alpha\"", "alsa_output.usb-HyperX", &bass());
        assert!(config.contains("node.name = \"earmuff_eq_03f0_098d\""));
        assert!(config.contains("target.object = \"alsa_output.usb-HyperX\""));
        assert!(config.contains("node.description = \"Cloud \\\"Alpha\\\" (Earmuff equalizer)\""));
        assert!(
            config.contains(
                "name = eq_band_1 label = bq_peaking control = { \"Freq\" = 32.0 \"Q\" = 1.41 \"Gain\" = 6.0 }"
            )
        );
        assert!(config.contains(
            "name = eq_band_10 label = bq_peaking control = { \"Freq\" = 16000.0 \"Q\" = 1.41 \"Gain\" = -2.5 }"
        ));
        assert!(config.contains("{ output = \"eq_band_9:Out\" input = \"eq_band_10:In\" }"));
        assert_eq!(config.matches("output = ").count(), 10, "preamp to band 1, then band to band");
        assert!(config.contains("outputs = [ \"eq_band_10:Out\" ]"));
    }
}
