use objc2::rc::Retained;
use objc2_core_audio::{
    AudioObjectGetPropertyData, AudioObjectID, AudioObjectPropertyAddress,
    AudioObjectPropertyScope, AudioObjectPropertySelector, kAudioDevicePropertyDeviceUID,
    kAudioDevicePropertyNominalSampleRate, kAudioHardwarePropertyDefaultOutputDevice,
    kAudioHardwarePropertyTranslatePIDToProcessObject, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject,
};
use objc2_foundation::NSString;
use std::ffi::c_void;
use std::mem::MaybeUninit;
use std::ptr::NonNull;

pub const SYSTEM_OBJECT: AudioObjectID = kAudioObjectSystemObject as AudioObjectID;
const GLOBAL: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal;

/// Reads a fixed-size property. `T` must match the property's data type exactly.
unsafe fn get<T, Q>(
    object: AudioObjectID,
    selector: AudioObjectPropertySelector,
    qualifier: Option<&Q>,
) -> Option<T> {
    let mut address = AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: GLOBAL,
        mElement: kAudioObjectPropertyElementMain,
    };
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

/// The Core Audio process object for this process, used to keep our own output out of the tap.
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
