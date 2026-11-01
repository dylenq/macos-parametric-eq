use objc2::rc::Retained;
use objc2_core_audio::{
    AudioDeviceIOProcID, AudioHardwareIOProcStreamUsage, AudioObjectGetPropertyData,
    AudioObjectGetPropertyDataSize, AudioObjectID, AudioObjectPropertyAddress,
    AudioObjectPropertyScope, AudioObjectPropertySelector, AudioObjectSetPropertyData,
    kAudioDevicePropertyDeviceUID, kAudioDevicePropertyIOProcStreamUsage,
    kAudioDevicePropertyNominalSampleRate, kAudioDevicePropertyPreferredChannelsForStereo,
    kAudioDevicePropertyStreams, kAudioHardwarePropertyDefaultOutputDevice,
    kAudioHardwarePropertyTranslatePIDToProcessObject, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyScopeInput,
    kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject, kAudioStreamPropertyVirtualFormat,
};
use objc2_core_audio_types::{
    AudioStreamBasicDescription, kAudioFormatFlagIsBigEndian, kAudioFormatFlagIsFloat,
    kAudioFormatFlagIsNonMixable, kAudioFormatLinearPCM,
};
use objc2_foundation::NSString;
use std::ffi::c_void;
use std::mem::MaybeUninit;
use std::ptr::NonNull;

pub const SYSTEM_OBJECT: AudioObjectID = kAudioObjectSystemObject as AudioObjectID;
const GLOBAL: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal;
const INPUT: AudioObjectPropertyScope = kAudioObjectPropertyScopeInput;
const OUTPUT: AudioObjectPropertyScope = kAudioObjectPropertyScopeOutput;

fn address(selector: AudioObjectPropertySelector, scope: AudioObjectPropertyScope) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain,
    }
}

/// Reads a fixed-size property. `T` must match the property's data type exactly.
unsafe fn get<T, Q>(
    object: AudioObjectID,
    selector: AudioObjectPropertySelector,
    qualifier: Option<&Q>,
) -> Option<T> {
    unsafe { get_in::<T, Q>(object, selector, GLOBAL, qualifier) }
}

/// Reads a fixed-size property in `scope`. `T` must match the property's data type exactly.
unsafe fn get_in<T, Q>(
    object: AudioObjectID,
    selector: AudioObjectPropertySelector,
    scope: AudioObjectPropertyScope,
    qualifier: Option<&Q>,
) -> Option<T> {
    let mut address = address(selector, scope);
    let mut size = size_of::<T>() as u32;
    let mut value = MaybeUninit::<T>::zeroed();
    let (qualifier_size, qualifier_ptr) = match qualifier {
        Some(q) => (size_of::<Q>() as u32, q as *const Q as *const c_void),
        None => (0, std::ptr::null()),
    };
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&mut address),
            qualifier_size,
            qualifier_ptr,
            NonNull::from(&mut size),
            NonNull::new_unchecked(value.as_mut_ptr().cast()),
        )
    };
    (status == 0 && size as usize == size_of::<T>()).then(|| unsafe { value.assume_init() })
}

/// Reads a property that is an array of object IDs, such as a device's streams.
fn get_ids(object: AudioObjectID, selector: AudioObjectPropertySelector, scope: AudioObjectPropertyScope) -> Vec<AudioObjectID> {
    let mut address = address(selector, scope);
    let mut size = 0u32;
    let status = unsafe {
        AudioObjectGetPropertyDataSize(object, NonNull::from(&mut address), 0, std::ptr::null(), NonNull::from(&mut size))
    };
    let count = size as usize / size_of::<AudioObjectID>();
    if status != 0 || count == 0 {
        return Vec::new();
    }
    let mut ids = vec![0 as AudioObjectID; count];
    let mut size = (count * size_of::<AudioObjectID>()) as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&mut address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new_unchecked(ids.as_mut_ptr().cast()),
        )
    };
    if status != 0 {
        return Vec::new();
    }
    // The list may have shrunk between the two calls.
    ids.truncate(size as usize / size_of::<AudioObjectID>());
    ids
}

/// Reads a CFString property, which Core Audio hands back retained.
fn get_string(object: AudioObjectID, selector: AudioObjectPropertySelector) -> Option<Retained<NSString>> {
    let raw: *mut NSString = unsafe { get::<_, ()>(object, selector, None)? };
    unsafe { Retained::from_raw(raw) }
}

pub fn default_output_device() -> Option<AudioObjectID> {
    let id: AudioObjectID =
        unsafe { get::<_, ()>(SYSTEM_OBJECT, kAudioHardwarePropertyDefaultOutputDevice, None)? };
    (id != 0).then_some(id)
}

pub fn sample_rate(device: AudioObjectID) -> Option<f64> {
    let rate: f64 = unsafe { get::<_, ()>(device, kAudioDevicePropertyNominalSampleRate, None)? };
    (rate > 0.0).then_some(rate)
}

pub fn device_uid(device: AudioObjectID) -> Option<Retained<NSString>> {
    get_string(device, kAudioDevicePropertyDeviceUID)
}

pub fn device_name(device: AudioObjectID) -> Option<String> {
    get_string(device, kAudioObjectPropertyName).map(|s| s.to_string())
}

/// The channels, numbered from 1, that the device plays stereo on: what Audio MIDI Setup's
/// Configure Speakers chose, usually 1 and 2.
pub fn stereo_channels(device: AudioObjectID) -> Option<[u32; 2]> {
    unsafe { get_in::<[u32; 2], ()>(device, kAudioDevicePropertyPreferredChannelsForStereo, OUTPUT, None) }
}

/// The format each output stream of the device does its IO in, which is what an IOProc
/// on the device reads and writes.
pub fn output_formats(device: AudioObjectID) -> Vec<AudioStreamBasicDescription> {
    get_ids(device, kAudioDevicePropertyStreams, OUTPUT)
        .into_iter()
        .filter_map(|stream| unsafe {
            get::<AudioStreamBasicDescription, ()>(stream, kAudioStreamPropertyVirtualFormat, None)
        })
        .collect()
}

/// Whether samples in this format are native 32-bit floats, the only kind the IOProc writes.
/// Encoded audio passed through to a receiver, for instance, is not.
pub fn is_float32(format: &AudioStreamBasicDescription) -> bool {
    let flags = format.mFormatFlags;
    format.mFormatID == kAudioFormatLinearPCM
        && flags & kAudioFormatFlagIsFloat != 0
        && flags & (kAudioFormatFlagIsBigEndian | kAudioFormatFlagIsNonMixable) == 0
        && format.mBitsPerChannel == 32
}

/// Lets the IOProc `proc_id` on `device` read only the device's last input stream. On an
/// aggregate of an output device and a tap, that is the tap: the output device's own
/// inputs, such as a headset's microphone, then stay off. Running them would switch
/// Bluetooth headphones to their low-quality call mode and show the microphone in use.
pub fn read_last_input_only(device: AudioObjectID, proc_id: AudioDeviceIOProcID) -> bool {
    let streams = get_ids(device, kAudioDevicePropertyStreams, INPUT).len();
    let Some(proc_id) = proc_id else {
        return false;
    };
    if streams < 2 {
        return true;
    }
    // The structure ends in a flag for each stream, so it is laid out by hand.
    let count_at = std::mem::offset_of!(AudioHardwareIOProcStreamUsage, mNumberStreams);
    let flags_at = std::mem::offset_of!(AudioHardwareIOProcStreamUsage, mStreamIsOn);
    let size = flags_at + streams * size_of::<u32>();
    let mut usage = vec![0u64; size.div_ceil(size_of::<u64>())];
    let base = usage.as_mut_ptr().cast::<u8>();
    unsafe {
        base.cast::<*const c_void>().write(proc_id as *const c_void);
        base.add(count_at).cast::<u32>().write(streams as u32);
        let flags = base.add(flags_at).cast::<u32>();
        for stream in 0..streams {
            flags.add(stream).write((stream == streams - 1) as u32);
        }
    }
    let mut address = address(kAudioDevicePropertyIOProcStreamUsage, INPUT);
    let status = unsafe {
        AudioObjectSetPropertyData(
            device,
            NonNull::from(&mut address),
            0,
            std::ptr::null(),
            size as u32,
            NonNull::new_unchecked(base.cast()),
        )
    };
    status == 0
}

/// The output channels for left and right, counted from 0: the device's speaker pair when
/// it names two different channels the device has, else the first two. A device with one
/// channel gets no right channel, and so a mono mix.
pub fn speaker_channels(preferred: Option<[u32; 2]>, channels: usize) -> (usize, Option<usize>) {
    if let Some([left, right]) = preferred.map(|p| p.map(|c| c as usize))
        && left != right
        && (1..=channels).contains(&left)
        && (1..=channels).contains(&right)
    {
        return (left - 1, Some(right - 1));
    }
    (0, (channels != 1).then_some(1))
}

/// Which output stream, counted from 0, carries output channel `channel` (counted from 0
/// across the streams in order), given how many channels each stream has.
pub fn stream_of_channel(streams: &[usize], channel: usize) -> Option<usize> {
    let mut first = 0;
    for (index, &count) in streams.iter().enumerate() {
        if channel < first + count {
            return Some(index);
        }
        first += count;
    }
    None
}

/// The process object for this process, used to keep our own output out of the tap.
pub fn own_process_object() -> Option<AudioObjectID> {
    let pid = std::process::id() as i32;
    let id: AudioObjectID = unsafe {
        get(
            SYSTEM_OBJECT,
            kAudioHardwarePropertyTranslatePIDToProcessObject,
            Some(&pid),
        )?
    };
    (id != 0).then_some(id)
}

/// An OSStatus as Core Audio's four-character code where it is one ('!dev'), else a number.
pub fn os_status(status: i32) -> String {
    let bytes = status.to_be_bytes();
    if bytes.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
        format!("'{}'", String::from_utf8_lossy(&bytes))
    } else {
        status.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speakers_go_where_the_device_says() {
        assert_eq!(speaker_channels(Some([1, 2]), 2), (0, Some(1)));
        assert_eq!(speaker_channels(Some([3, 4]), 8), (2, Some(3)));
        assert_eq!(speaker_channels(Some([4, 3]), 8), (3, Some(2)));
        // Unknown, out of range, or one channel twice: the first two.
        assert_eq!(speaker_channels(None, 2), (0, Some(1)));
        assert_eq!(speaker_channels(Some([3, 4]), 2), (0, Some(1)));
        assert_eq!(speaker_channels(Some([0, 1]), 2), (0, Some(1)));
        assert_eq!(speaker_channels(Some([1, 1]), 2), (0, Some(1)));
        // A mono device, and a device whose channels could not be counted.
        assert_eq!(speaker_channels(Some([1, 2]), 1), (0, None));
        assert_eq!(speaker_channels(None, 0), (0, Some(1)));
    }

    #[test]
    fn the_tapped_stream_is_the_one_with_the_speakers() {
        assert_eq!(stream_of_channel(&[2], 0), Some(0));
        assert_eq!(stream_of_channel(&[8], 3), Some(0));
        // An interface with two stereo streams, its speakers on channels 3 and 4.
        assert_eq!(stream_of_channel(&[2, 2], 2), Some(1));
        assert_eq!(stream_of_channel(&[2, 2], 1), Some(0));
        assert_eq!(stream_of_channel(&[2], 2), None);
        assert_eq!(stream_of_channel(&[], 0), None);
    }

    #[test]
    fn only_native_float_samples_are_processed() {
        let float = AudioStreamBasicDescription {
            mSampleRate: 48_000.0,
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: kAudioFormatFlagIsFloat | (1 << 3),
            mBytesPerPacket: 8,
            mFramesPerPacket: 1,
            mBytesPerFrame: 8,
            mChannelsPerFrame: 2,
            mBitsPerChannel: 32,
            mReserved: 0,
        };
        assert!(is_float32(&float));
        assert!(!is_float32(&AudioStreamBasicDescription { mFormatFlags: 1 << 2, ..float }));
        assert!(!is_float32(&AudioStreamBasicDescription { mBitsPerChannel: 64, ..float }));
        assert!(!is_float32(&AudioStreamBasicDescription { mFormatID: u32::from_be_bytes(*b"ac-3"), ..float }));
        assert!(!is_float32(&AudioStreamBasicDescription { mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsNonMixable, ..float }));
    }

    #[test]
    fn status_codes_read_as_text() {
        assert_eq!(os_status(i32::from_be_bytes(*b"!dev")), "'!dev'");
        assert_eq!(os_status(i32::from_be_bytes(*b"nope")), "'nope'");
        assert_eq!(os_status(-50), "-50");
    }
}
