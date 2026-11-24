//! Analysis of the original recording; the gain is applied only by the player.
use crate::{Error, Result};
use serde_json::{Value, json};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::UNIX_EPOCH;

pub const TARGET_LUFS: f64 = -20.0;
pub const PEAK_CEILING_DBFS: f64 = -1.0;
/// EBU R 128 absolute gate: no measured programme reads below this.
const GATE_LUFS: f64 = -70.0;
/// The provisional lens reads only the opening of the recording.
const OPENING_SECONDS: f64 = 30.0;
/// True peak is most of the analysis cost; long recordings measure it over
/// parallel time slices beside one whole-file loudness pass.
const PEAK_SLICES: usize = 4;
const SLICED_FROM_SECONDS: f64 = 120.0;
const SLICE_OVERLAP_SECONDS: f64 = 0.05;

pub fn playback_gain(integrated_lufs: f64, true_peak_dbfs: f64) -> f64 {
    if !integrated_lufs.is_finite() || !true_peak_dbfs.is_finite() {
        return 1.0;
    }
    let boost = (TARGET_LUFS - integrated_lufs).max(0.0);
    10.0_f64.powf(boost.min(PEAK_CEILING_DBFS - true_peak_dbfs) / 20.0)
}

pub(crate) fn stamp(path: &Path) -> Result<Value> {
    let metadata = fs::metadata(path).map_err(|error| {
        Error::Invalid(format!(
            "Cannot inspect recording {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(Error::Invalid(format!(
            "Recording is not a file: {}",
            path.display()
        )));
    }
    let modified = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Ok(
        json!({"path":path,"size":metadata.len(),"mtime_ns":modified.as_nanos().to_string(),"analysis_version":1}),
    )
}

/// The whole-file lens is reused only if it is a completed measurement whose
/// gain is the one its own numbers give; anything else is measured again.
pub(crate) fn cache_matches(record: &Value, path: &Path) -> bool {
    stamp(path).is_ok_and(|stamp| record["_playback_stamp"] == stamp) && cached_gain_valid(record)
}

fn finite(value: &Value) -> Option<Option<f64>> {
    match value {
        Value::Null => Some(None),
        value => value.as_f64().filter(|n| n.is_finite()).map(Some),
    }
}

/// A lens (whole-file or the opening window) agrees with its own numbers.
fn lens_valid(state: Option<&str>, lens: &Value, gain: Option<f64>) -> bool {
    let Some(gain) = gain.filter(|gain| gain.is_finite() && *gain > 0.0) else {
        return false;
    };
    match (
        state,
        finite(&lens["integrated_lufs"]),
        finite(&lens["true_peak_dbfs"]),
    ) {
        (Some("measured"), Some(Some(integrated)), Some(Some(peak))) => {
            integrated >= GATE_LUFS && (gain - playback_gain(integrated, peak)).abs() <= 1e-9 * gain
        }
        (Some("measured"), Some(None), Some(Some(_))) | (Some("silent"), Some(_), Some(None)) => {
            gain == 1.0
        }
        _ => false,
    }
}

fn cached_gain_valid(record: &Value) -> bool {
    lens_valid(
        record["loudness"]["state"].as_str(),
        &record["loudness"],
        record["playback_gain"].as_f64(),
    )
}

/// The opening-window lens cached beside the whole-file one, for the same file.
pub(crate) fn provisional_matches(record: &Value, stamp: &Value) -> Option<f64> {
    let lens = &record["loudness"]["provisional"];
    let gain = lens["gain"].as_f64();
    (record["_playback_stamp"] == *stamp && lens_valid(lens["state"].as_str(), lens, gain))
        .then(|| gain.unwrap_or(1.0))
}

fn summary_number(output: &str, label: &str) -> Result<Option<f64>> {
    let summary = output
        .rsplit_once("Summary:")
        .map(|(_, summary)| summary)
        .ok_or_else(|| Error::Invalid("Recording analysis returned no summary".into()))?;
    let field = summary
        .lines()
        .find_map(|line| line.trim().strip_prefix(label))
        .and_then(|field| field.split_whitespace().next())
        .ok_or_else(|| Error::Invalid(format!("Recording analysis lacks {label}")))?;
    let value = field
        .parse::<f64>()
        .map_err(|_| Error::Invalid(format!("Recording analysis has invalid {label}")))?;
    Ok(value.is_finite().then_some(value))
}

#[cfg(test)]
fn parse_measurement(output: &str) -> Result<(Option<f64>, Option<f64>)> {
    Ok((
        summary_number(output, "I:")?,
        summary_number(output, "Peak:")?,
    ))
}

/// Length of a recording in milliseconds, read from the file itself: the WAV
/// header when it is one (a few bytes, never the whole file), else ffprobe.
pub(crate) fn duration_ms(path: &Path) -> Option<i64> {
    wav_duration_ms(path).or_else(|| probe_seconds(path).map(|s| (s * 1000.0) as i64))
}

fn wav_duration_ms(path: &Path) -> Option<i64> {
    let mut file = fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let mut header = vec![0u8; 64 * 1024];
    let read = file.read(&mut header).ok()?;
    header.truncate(read);
    if header.len() < 12 || &header[..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return None;
    }
    let mut offset = 12usize;
    let mut byte_rate = None;
    while offset + 8 <= header.len() {
        let id = &header[offset..offset + 4];
        let size = u32::from_le_bytes(header[offset + 4..offset + 8].try_into().ok()?) as u64;
        let body = offset + 8;
        if id == b"fmt " && body + 12 <= header.len() {
            byte_rate = Some(u32::from_le_bytes(
                header[body + 8..body + 12].try_into().ok()?,
            ));
        }
        if id == b"data" {
            let rate = u64::from(byte_rate.filter(|rate| *rate > 0)?);
            // A recorder that stopped without rewriting its header leaves the
            // declared size empty or oversized; the file holds the real count.
            let available = length.saturating_sub(body as u64);
            let bytes = if size == 0 || size > available {
                available
            } else {
                size
            };
            return i64::try_from(bytes * 1000 / rate).ok();
        }
        offset = body.checked_add(usize::try_from(size + (size & 1)).ok()?)?;
    }
    None
}

fn probe_seconds(path: &Path) -> Option<f64> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-threads",
            "1",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
}

/// One ebur128 pass over `window` (start, length; no length reads to the end).
fn analysis(path: &Path, window: Option<(f64, Option<f64>)>, filter: &str) -> Command {
    let mut command = Command::new("ffmpeg");
    command.args(["-hide_banner", "-nostdin", "-nostats", "-threads", "1"]);
    if let Some((start, length)) = window {
        command.args(["-ss", &format!("{start:.3}")]);
        if let Some(length) = length {
            command.args(["-t", &format!("{length:.3}")]);
        }
    }
    command
        .arg("-i")
        .arg(path)
        .args([
            "-map",
            "0:a:0",
            "-filter_threads",
            "1",
            "-af",
            filter,
            "-f",
            "null",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
}

/// Run every pass at once; the first failure ends the others (the analysis is
/// cancelled as a whole and nothing partial is used).
fn run_passes(path: &Path, commands: Vec<Command>) -> Result<Vec<String>> {
    let mut children = Vec::with_capacity(commands.len());
    let mut failure = None;
    for mut command in commands {
        match command.spawn() {
            Ok(child) => children.push(child),
            Err(error) => {
                failure = Some(format!(
                    "Cannot analyze recording {}: {error}",
                    path.display()
                ));
                break;
            }
        }
    }
    let outputs = std::thread::scope(|scope| {
        let readers: Vec<_> = children
            .iter_mut()
            .map(|child| {
                let stderr = child.stderr.take();
                scope.spawn(move || {
                    let mut bytes = Vec::new();
                    if let Some(mut stderr) = stderr {
                        let _ = stderr.read_to_end(&mut bytes);
                    }
                    String::from_utf8_lossy(&bytes).into_owned()
                })
            })
            .collect();
        for child in &mut children {
            if failure.is_some() {
                break;
            }
            match child.wait() {
                Ok(status) if status.success() => {}
                Ok(status) => {
                    failure = Some(format!(
                        "Recording analysis failed for {} ({status})",
                        path.display()
                    ))
                }
                Err(error) => failure = Some(format!("Recording analysis failed: {error}")),
            }
        }
        if failure.is_some() {
            for child in &mut children {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        readers
            .into_iter()
            .map(|reader| reader.join().unwrap_or_default())
            .collect::<Vec<_>>()
    });
    match failure {
        Some(message) => Err(Error::Invalid(message)),
        None => Ok(outputs),
    }
}

fn lens(integrated: Option<f64>, peak: Option<f64>) -> (Value, f64) {
    let silent = peak.is_none();
    let gain = integrated
        .zip(peak)
        .map_or(1.0, |(integrated, peak)| playback_gain(integrated, peak));
    (
        json!({"state":if silent {"silent"} else {"measured"},
        "integrated_lufs":integrated,"true_peak_dbfs":peak,
        "target_lufs":TARGET_LUFS,"peak_ceiling_dbfs":PEAK_CEILING_DBFS,
        "output_lufs":integrated.map(|value| value + 20.0 * gain.log10()),
        "peak_limited":integrated.is_some_and(|value| value < TARGET_LUFS && value + 20.0 * gain.log10() < TARGET_LUFS - 0.01)}),
        gain,
    )
}

fn unchanged(path: &Path, before: &Value) -> Result<()> {
    if stamp(path)? != *before {
        return Err(Error::Invalid(format!(
            "Recording changed during analysis: {}",
            path.display()
        )));
    }
    Ok(())
}

/// The provisional lens: one pass over the opening window of the recording.
pub(crate) fn measure_opening(path: &Path) -> Result<(Value, f64)> {
    let before = stamp(path)?;
    let outputs = run_passes(
        path,
        vec![analysis(
            path,
            Some((0.0, Some(OPENING_SECONDS))),
            "ebur128=peak=true",
        )],
    )?;
    unchanged(path, &before)?;
    let (mut value, gain) = lens(
        summary_number(&outputs[0], "I:")?,
        summary_number(&outputs[0], "Peak:")?,
    );
    value["window_seconds"] = json!(OPENING_SECONDS);
    value["gain"] = json!(gain);
    Ok((value, gain))
}

/// The whole-file lens. Long recordings measure true peak over parallel time
/// slices beside one whole-file loudness pass.
pub(crate) fn measure(path: &Path) -> Result<(Value, f64)> {
    let before = stamp(path)?;
    let duration = duration_ms(path)
        .map(|ms| ms as f64 / 1000.0)
        .filter(|seconds| *seconds >= SLICED_FROM_SECONDS);
    let (integrated, peak) = if let Some(duration) = duration {
        let slice = duration / PEAK_SLICES as f64;
        let mut commands = vec![analysis(path, None, "ebur128")];
        for index in 0..PEAK_SLICES {
            let length = (index + 1 < PEAK_SLICES).then_some(slice + SLICE_OVERLAP_SECONDS);
            commands.push(analysis(
                path,
                Some((slice * index as f64, length)),
                "ebur128=peak=true",
            ));
        }
        let outputs = run_passes(path, commands)?;
        let integrated = summary_number(&outputs[0], "I:")?;
        let mut peak: Option<f64> = None;
        for output in &outputs[1..] {
            if let Some(slice_peak) = summary_number(output, "Peak:")? {
                peak = Some(peak.map_or(slice_peak, |peak| peak.max(slice_peak)));
            }
        }
        (integrated, peak)
    } else {
        let outputs = run_passes(path, vec![analysis(path, None, "ebur128=peak=true")])?;
        (
            summary_number(&outputs[0], "I:")?,
            summary_number(&outputs[0], "Peak:")?,
        )
    };
    unchanged(path, &before)?;
    Ok(lens(integrated, peak))
}

/// Select only the player's output stream, never an audio device or another app.
fn player_stream(objects: &Value, player_pid: u32) -> Result<Option<(u32, usize)>> {
    fn number(value: &Value) -> Option<u64> {
        value.as_u64().or_else(|| value.as_str()?.parse().ok())
    }
    let objects = objects
        .as_array()
        .ok_or_else(|| Error::Invalid("Invalid playback graph".into()))?;
    let clients: Vec<u64> = objects
        .iter()
        .filter(|object| {
            object["type"] == "PipeWire:Interface:Client"
                && number(&object["info"]["props"]["pipewire.sec.pid"])
                    == Some(u64::from(player_pid))
        })
        .filter_map(|object| number(&object["id"]))
        .collect();
    let streams: Vec<&Value> = objects
        .iter()
        .filter(|object| {
            object["type"] == "PipeWire:Interface:Node"
                && object["info"]["props"]["media.class"] == "Stream/Output/Audio"
                && number(&object["info"]["props"]["client.id"])
                    .is_some_and(|id| clients.contains(&id))
        })
        .collect();
    match streams.as_slice() {
        [] => Ok(None),
        [stream] => {
            let id = number(&stream["id"])
                .and_then(|id| u32::try_from(id).ok())
                .ok_or_else(|| Error::Invalid("Invalid player stream identity".into()))?;
            let channels = stream["info"]["params"]["Props"]
                .as_array()
                .into_iter()
                .flatten()
                .find_map(|props| {
                    props["channelVolumes"]
                        .as_array()
                        .filter(|v| !v.is_empty())
                        .map(Vec::len)
                })
                .or_else(|| {
                    stream["info"]["params"]["EnumFormat"][0]["channels"]
                        .as_u64()
                        .map(|n| n as usize)
                })
                .filter(|channels| (1..=64).contains(channels))
                .ok_or_else(|| Error::Invalid("Player channel layout is not ready".into()))?;
            Ok(Some((id, channels)))
        }
        _ => Err(Error::Invalid(
            "More than one output belongs to this player; no gain applied".into(),
        )),
    }
}

/// Per-stream PipeWire lens. The selected process must be the UI's own player.
/// The stream disappears with that player; no device or saved settings change.
pub fn apply_playback_gain(player_pid: u32, gain: f64) -> Result<Value> {
    if player_pid == 0 || !gain.is_finite() || gain <= 0.0 {
        return Err(Error::Invalid("Invalid playback gain request".into()));
    }
    let graph = Command::new("pw-dump").stdin(Stdio::null()).output()?;
    if !graph.status.success() {
        return Err(Error::Invalid(
            "Cannot inspect the player's playback stream".into(),
        ));
    }
    let objects = serde_json::from_slice(&graph.stdout)?;
    let Some((node, channels)) = player_stream(&objects, player_pid)? else {
        return Ok(json!({"state":"waiting"}));
    };
    // The graph advertises channelmix.max-volume=10. Raise only this stream's
    // software ceiling when needed; the recording's true peak caps our gain.
    let props = json!({"volume":1.0,"channelVolumes":vec![gain;channels],
        "params":["channelmix.max-volume",gain.max(10.0)]});
    let output = Command::new("pw-cli")
        .args(["set-param", &node.to_string(), "Props", &props.to_string()])
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(Error::Invalid(
            "Cannot apply the player's playback gain".into(),
        ));
    }
    Ok(json!({"state":"applied","node_id":node,"channels":channels,"playback_gain":gain}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comfortable_recordings_stay_at_unity_and_quiet_peaks_cannot_clip() {
        assert_eq!(playback_gain(-18.0, -3.0), 1.0);
        assert!((20.0 * playback_gain(-47.7, -29.7).log10() - 27.7).abs() < 1e-9);
        assert!((-2.0 + 20.0 * playback_gain(-35.0, -2.0).log10() + 1.0).abs() < 1e-9);
        assert_eq!(playback_gain(f64::NAN, -2.0), 1.0);
    }

    #[test]
    fn summary_parser_reads_final_measurements_and_accepts_silence() {
        assert_eq!(
            parse_measurement("Summary:\n I: -47.7 LUFS\n Peak: -29.7 dBFS\n").unwrap(),
            (Some(-47.7), Some(-29.7))
        );
        assert_eq!(
            parse_measurement("Summary:\n I: -70.0 LUFS\n Peak: -inf dBFS\n").unwrap(),
            (Some(-70.0), None)
        );
        assert!(parse_measurement("Summary:\n I: bad LUFS\n Peak: -2 dBFS").is_err());
    }

    #[test]
    fn stream_selection_uses_server_pid_and_never_selects_a_device_or_other_player() {
        let graph = json!([
            {"id":1,"type":"PipeWire:Interface:Client","info":{"props":{"pipewire.sec.pid":42}}},
            {"id":2,"type":"PipeWire:Interface:Client","info":{"props":{"pipewire.sec.pid":43}}},
            {"id":3,"type":"PipeWire:Interface:Node","info":{"props":{"client.id":1,"media.class":"Audio/Sink"}}},
            {"id":4,"type":"PipeWire:Interface:Node","info":{"props":{"client.id":2,"media.class":"Stream/Output/Audio"}}},
            {"id":5,"type":"PipeWire:Interface:Node","info":{"props":{"client.id":1,"media.class":"Stream/Output/Audio"},"params":{"EnumFormat":[{"channels":2}]}}}
        ]);
        assert_eq!(player_stream(&graph, 42).unwrap(), Some((5, 2)));
        assert_eq!(player_stream(&graph, 44).unwrap(), None);
        let mut ambiguous = graph;
        let duplicate = ambiguous[4].clone();
        ambiguous.as_array_mut().unwrap().push(duplicate);
        assert!(player_stream(&ambiguous, 42).is_err());
    }

    #[test]
    fn only_a_completed_self_consistent_measurement_is_reused() {
        let measured = |gain: Value| {
            json!({"loudness":{"state":"measured","integrated_lufs":-63.8,"true_peak_dbfs":-41.5},
                "playback_gain":gain})
        };
        assert!(cached_gain_valid(&measured(json!(playback_gain(
            -63.8, -41.5
        )))));
        // Tampered or stale gains, failures and malformed lenses are measured again.
        assert!(!cached_gain_valid(&measured(json!(1.0e6))));
        assert!(!cached_gain_valid(&measured(json!("105.9"))));
        assert!(!cached_gain_valid(&measured(Value::Null)));
        assert!(!cached_gain_valid(
            &json!({"loudness":{"state":"unavailable","reason":"cancelled"},"playback_gain":1.0})
        ));
        assert!(!cached_gain_valid(
            &json!({"loudness":{"state":"measuring","provisional":{"state":"measured"}},"playback_gain":1.0})
        ));
        assert!(!cached_gain_valid(
            &json!({"loudness":"measured","playback_gain":1.0})
        ));
        assert!(!cached_gain_valid(&json!({"loudness":{"state":"measured",
            "integrated_lufs":-200.0,"true_peak_dbfs":-199.0},"playback_gain":playback_gain(-200.0, -199.0)})));
        assert!(cached_gain_valid(&json!({"loudness":{"state":"silent",
            "integrated_lufs":-70.0,"true_peak_dbfs":null},"playback_gain":1.0})));
        assert!(!cached_gain_valid(&json!({"loudness":{"state":"silent",
            "integrated_lufs":-70.0,"true_peak_dbfs":null},"playback_gain":3.0})));
    }

    #[test]
    fn provisional_lens_is_reused_only_for_the_same_file() {
        let gain = playback_gain(-63.9, -48.6);
        let record = json!({"_playback_stamp":{"size":1},"loudness":{"state":"measuring",
            "provisional":{"state":"measured","integrated_lufs":-63.9,"true_peak_dbfs":-48.6,"gain":gain}}});
        assert_eq!(provisional_matches(&record, &json!({"size":1})), Some(gain));
        assert_eq!(provisional_matches(&record, &json!({"size":2})), None);
        let mut tampered = record.clone();
        tampered["loudness"]["provisional"]["gain"] = json!(1.0e6);
        assert_eq!(provisional_matches(&tampered, &json!({"size":1})), None);
    }

    #[test]
    fn wav_header_gives_the_length_without_reading_the_samples() {
        let dir = std::env::temp_dir().join(format!("soliton-wav-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.wav");
        let rate: u32 = 48_000 * 2 * 2;
        let data = vec![0u8; rate as usize * 3 / 2];
        let mut bytes = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(48_000u32.to_le_bytes());
        bytes.extend(rate.to_le_bytes());
        bytes.extend(4u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        // An unfinished header (size 0) still yields the file's real length.
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(&data);
        fs::write(&path, &bytes).unwrap();
        assert_eq!(wav_duration_ms(&path), Some(1500));
        fs::write(dir.join("not.wav"), b"plain text").unwrap();
        assert_eq!(wav_duration_ms(&dir.join("not.wav")), None);
        fs::remove_file(&path).unwrap();
        fs::remove_file(dir.join("not.wav")).unwrap();
        fs::remove_dir(&dir).unwrap();
    }
}
