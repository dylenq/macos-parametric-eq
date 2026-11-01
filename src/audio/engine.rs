use super::aggregate::Aggregate;
use super::devices;
use super::tap::Tap;
use crate::dsp::chain::{Chain, ChainParams};
use objc2_core_audio::{
    AudioDeviceCreateIOProcID, AudioDeviceDestroyIOProcID, AudioDeviceIOProcID, AudioDeviceStart,
    AudioDeviceStop, AudioObjectID,
};
use objc2_core_audio_types::{AudioBuffer, AudioBufferList, AudioTimeStamp};
use std::ffi::c_void;
use std::os::raw::c_int;
use std::panic::AssertUnwindSafe;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Peak levels since the last read, written by the audio thread.
#[derive(Default)]
pub struct Meters {
    input: AtomicU32,
    output: AtomicU32,
    /// Processing panicked. The IOProc plays silence from then on, until the engine is replaced.
    failed: AtomicBool,
}

impl Meters {
    fn raise(slot: &AtomicU32, peak: f32) {
        if peak > f32::from_bits(slot.load(Ordering::Relaxed)) {
            slot.store(peak.to_bits(), Ordering::Relaxed);
        }
    }

    /// Returns (input, output) linear peaks and resets them.
    pub fn take(&self) -> (f32, f32) {
        (
            f32::from_bits(self.input.swap(0, Ordering::Relaxed)),
            f32::from_bits(self.output.swap(0, Ordering::Relaxed)),
        )
    }

    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }
}

/// State owned by the IOProc.
struct Callback {
    chain: Chain,
    params: rtrb::Consumer<ChainParams>,
    meters: Arc<Meters>,
    /// Output channels for left and right, counted from 0. No right channel means a mono mix.
    left: usize,
    right: Option<usize>,
}

/// Running tap → EQ → output pipeline. Dropping it restores normal system audio.
pub struct Engine {
    // Declaration order is teardown order after `drop` stops the IOProc.
    aggregate: Aggregate,
    _tap: Tap,
    proc_id: AudioDeviceIOProcID,
    callback: *mut Callback,
    params: rtrb::Producer<ChainParams>,
    pub meters: Arc<Meters>,
    pub output_device: AudioObjectID,
    pub sample_rate: f64,
}

// The callback pointer is only dereferenced by the IOProc and freed in `drop` after the
// IOProc has stopped, so the handle can move between threads.
unsafe impl Send for Engine {}

impl Engine {
    /// Starts processing audio headed for `output_device`. `make_params` gets the sample
    /// rate the filters will run at.
    pub fn start(
        output_device: AudioObjectID,
        make_params: impl FnOnce(f64) -> ChainParams,
    ) -> Result<Engine, String> {
        let output_uid =
            devices::device_uid(output_device).ok_or("Could not read the output device")?;
        let tap = Tap::new()?;
        let aggregate = Aggregate::new(&output_uid, &tap)?;
        let sample_rate = devices::sample_rate(aggregate.id)
            .or_else(|| devices::sample_rate(output_device))
            .ok_or("Could not read the output sample rate")?;

        // The IOProc writes 32-bit floats. Written into an output passing encoded audio
        // through, they would play as loud noise, so such an output is left alone. If the
        // formats cannot be read at all, the output is assumed to be the usual float one.
        let formats = devices::output_formats(aggregate.id);
        if !formats.iter().all(devices::is_float32) {
            return Err("EQ can't process this output's audio format. If it passes encoded \
                        audio through to a receiver, switch that off in Audio MIDI Setup."
                .into());
        }
        let channels = formats.iter().map(|f| f.mChannelsPerFrame as usize).sum();
        let (left, right) = devices::speaker_channels(devices::stereo_channels(output_device), channels);

        let (producer, consumer) = rtrb::RingBuffer::new(32);
        let meters = Arc::new(Meters::default());
        let callback = Box::into_raw(Box::new(Callback {
            chain: Chain::new(sample_rate, &make_params(sample_rate)),
            params: consumer,
            meters: meters.clone(),
            left,
            right,
        }));

        let mut proc_id: AudioDeviceIOProcID = None;
        let mut status = unsafe {
            AudioDeviceCreateIOProcID(
                aggregate.id,
                Some(io_proc),
                callback.cast(),
                NonNull::from(&mut proc_id),
            )
        };
        if status == 0 {
            // Only the tap is read: the output device's own inputs stay off.
            if !devices::read_last_input_only(aggregate.id, proc_id) {
                eprintln!("could not limit audio input to the tap");
            }
            // With the tap set to start the aggregate, this waits until something plays.
            status = unsafe { AudioDeviceStart(aggregate.id, proc_id) };
            if status != 0 {
                unsafe { AudioDeviceDestroyIOProcID(aggregate.id, proc_id) };
            }
        }
        if status != 0 {
            drop(unsafe { Box::from_raw(callback) });
            return Err(start_error(status));
        }

        Ok(Engine {
            aggregate,
            _tap: tap,
            proc_id,
            callback,
            params: producer,
            meters,
            output_device,
            sample_rate,
        })
    }

    /// Hands new parameters to the audio thread. Returns false if the queue is full and
    /// the caller should retry.
    pub fn send(&mut self, params: ChainParams) -> bool {
        self.params.push(params).is_ok()
    }
}

/// Why the output would not start, in words where the code is a common one.
fn start_error(status: i32) -> String {
    let code = devices::os_status(status);
    let reason = match &status.to_be_bytes() {
        b"!hog" => "another app has exclusive use of the output",
        b"!dev" | b"nope" => "the output can't be used this way",
        b"stop" => "the output has stopped",
        _ => return format!("Could not start audio processing (error {code})"),
    };
    format!("Could not start audio processing: {reason} ({code})")
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe {
            // AudioDeviceStop returns only once the IOProc is no longer running.
            AudioDeviceStop(self.aggregate.id, self.proc_id);
            AudioDeviceDestroyIOProcID(self.aggregate.id, self.proc_id);
            drop(Box::from_raw(self.callback));
        }
    }
}

#[derive(Clone, Copy)]
struct Channel {
    data: *mut f32,
    stride: usize,
    frames: usize,
}

unsafe fn buffers<'a>(list: *mut AudioBufferList) -> &'a [AudioBuffer] {
    unsafe {
        let first = (&raw const (*list).mBuffers).cast::<AudioBuffer>();
        std::slice::from_raw_parts(first, (*list).mNumberBuffers as usize)
    }
}

fn usable(buffer: &AudioBuffer) -> bool {
    !buffer.mData.is_null() && buffer.mNumberChannels > 0
}

fn channel_count(buffers: &[AudioBuffer]) -> usize {
    buffers
        .iter()
        .filter(|b| usable(b))
        .map(|b| b.mNumberChannels as usize)
        .sum()
}

/// Finds channel `index` counting across all buffers, whether they are interleaved or not.
fn channel(buffers: &[AudioBuffer], mut index: usize) -> Option<Channel> {
    for buffer in buffers.iter().filter(|b| usable(b)) {
        let channels = buffer.mNumberChannels as usize;
        if index < channels {
            return Some(Channel {
                data: unsafe { buffer.mData.cast::<f32>().add(index) },
                stride: channels,
                frames: buffer.mDataByteSize as usize / (size_of::<f32>() * channels),
            });
        }
        index -= channels;
    }
    None
}

fn silence(buffers: &[AudioBuffer]) {
    for buffer in buffers.iter().filter(|b| usable(b)) {
        unsafe { std::ptr::write_bytes(buffer.mData.cast::<u8>(), 0, buffer.mDataByteSize as usize) };
    }
}

unsafe extern "C-unwind" fn io_proc(
    _device: AudioObjectID,
    _now: NonNull<AudioTimeStamp>,
    input: NonNull<AudioBufferList>,
    _input_time: NonNull<AudioTimeStamp>,
    output: NonNull<AudioBufferList>,
    _output_time: NonNull<AudioTimeStamp>,
    client: *mut c_void,
) -> c_int {
    let callback = unsafe { &mut *client.cast::<Callback>() };
    let input = unsafe { buffers(input.as_ptr()) };
    let output = unsafe { buffers(output.as_ptr()) };
    silence(output);
    if callback.meters.failed() {
        return 0;
    }
    // A panic must not unwind into Core Audio. Play silence instead, and let the window
    // know so it can start a fresh engine.
    if std::panic::catch_unwind(AssertUnwindSafe(|| process(callback, input, output))).is_err() {
        silence(output);
        callback.meters.failed.store(true, Ordering::Relaxed);
    }
    0
}

/// Runs the tap's stereo input through the chain into the output, which starts silent.
fn process(callback: &mut Callback, input: &[AudioBuffer], output: &[AudioBuffer]) {
    let mut latest = None;
    while let Ok(params) = callback.params.pop() {
        latest = Some(params);
    }
    if let Some(params) = latest {
        callback.chain.set_params(&params);
    }

    // The tap's stereo stream comes after any input streams of the output device itself.
    let in_channels = channel_count(input);
    let (Some(in_l), Some(in_r)) = (
        channel(input, in_channels.saturating_sub(2)),
        channel(input, in_channels.saturating_sub(1)),
    ) else {
        return;
    };
    let Some(out_l) = channel(output, callback.left).or_else(|| channel(output, 0)) else {
        return;
    };
    let out_r = callback.right.and_then(|right| channel(output, right));

    let frames = in_l
        .frames
        .min(in_r.frames)
        .min(out_l.frames)
        .min(out_r.map_or(usize::MAX, |c| c.frames));
    let (mut in_peak, mut out_peak) = (0f32, 0f32);
    for i in 0..frames {
        unsafe {
            let x = [*in_l.data.add(i * in_l.stride), *in_r.data.add(i * in_r.stride)];
            let y = callback.chain.tick(x);
            in_peak = in_peak.max(x[0].abs()).max(x[1].abs());
            out_peak = out_peak.max(y[0].abs()).max(y[1].abs());
            match out_r {
                Some(out_r) => {
                    *out_l.data.add(i * out_l.stride) = y[0];
                    *out_r.data.add(i * out_r.stride) = y[1];
                }
                None => *out_l.data.add(i * out_l.stride) = (y[0] + y[1]) * 0.5,
            }
        }
    }
    callback.chain.flush_denormals();
    Meters::raise(&callback.meters.input, in_peak);
    Meters::raise(&callback.meters.output, out_peak);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_errors_say_what_went_wrong() {
        let code = |s: &[u8; 4]| i32::from_be_bytes(*s);
        assert_eq!(
            start_error(code(b"!hog")),
            "Could not start audio processing: another app has exclusive use of the output ('!hog')"
        );
        assert_eq!(start_error(-50), "Could not start audio processing (error -50)");
    }
}
