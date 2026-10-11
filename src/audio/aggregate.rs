use super::tap::Tap;
use objc2::Message;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_audio::{
    AudioHardwareCreateAggregateDevice, AudioHardwareDestroyAggregateDevice, AudioObjectID,
    kAudioAggregateDeviceIsPrivateKey, kAudioAggregateDeviceIsStackedKey,
    kAudioAggregateDeviceMainSubDeviceKey, kAudioAggregateDeviceNameKey,
    kAudioAggregateDeviceSubDeviceListKey, kAudioAggregateDeviceTapAutoStartKey,
    kAudioAggregateDeviceTapListKey, kAudioAggregateDeviceUIDKey, kAudioSubDeviceUIDKey,
    kAudioSubTapDriftCompensationKey, kAudioSubTapUIDKey,
};
use objc2_core_foundation::CFDictionary;
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString, NSUUID, ns_string};
use std::ffi::CStr;
use std::ptr::NonNull;

fn any<T: Message>(object: Retained<T>) -> Retained<AnyObject> {
    unsafe { Retained::cast_unchecked(object) }
}

fn dictionary(entries: Vec<(&CStr, Retained<AnyObject>)>) -> Retained<NSDictionary<NSString, AnyObject>> {
    let (keys, objects): (Vec<_>, Vec<_>) = entries
        .into_iter()
        .map(|(key, object)| (NSString::from_str(key.to_str().unwrap()), object))
        .unzip();
    let keys: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    NSDictionary::from_retained_objects(&keys, &objects)
}

/// A private aggregate device that pairs the real output device (clock source) with the
/// tap as its input, so one IOProc can read the tap and write to the hardware.
pub struct Aggregate {
    pub id: AudioObjectID,
}

impl Aggregate {
    pub fn new(output_uid: &NSString, tap: &Tap) -> Result<Aggregate, String> {
        let yes = || any(NSNumber::new_bool(true));
        let no = || any(NSNumber::new_bool(false));

        let sub_device = dictionary(vec![(kAudioSubDeviceUIDKey, any(output_uid.retain()))]);
        let sub_tap = dictionary(vec![
            (kAudioSubTapUIDKey, any(tap.uid.clone())),
            (kAudioSubTapDriftCompensationKey, yes()),
        ]);
        let description = dictionary(vec![
            (kAudioAggregateDeviceNameKey, any(ns_string!("EQ").retain())),
            (kAudioAggregateDeviceUIDKey, any(NSUUID::new().UUIDString())),
            (kAudioAggregateDeviceMainSubDeviceKey, any(output_uid.retain())),
            (kAudioAggregateDeviceIsPrivateKey, yes()),
            (kAudioAggregateDeviceIsStackedKey, no()),
            (kAudioAggregateDeviceTapAutoStartKey, yes()),
            (
                kAudioAggregateDeviceSubDeviceListKey,
                any(NSArray::from_retained_slice(&[sub_device])),
            ),
            (
                kAudioAggregateDeviceTapListKey,
                any(NSArray::from_retained_slice(&[sub_tap])),
            ),
        ]);

        // NSDictionary is toll-free bridged to CFDictionary.
        let description: &CFDictionary =
            unsafe { &*(Retained::as_ptr(&description) as *const CFDictionary) };
        let mut id: AudioObjectID = 0;
        let status =
            unsafe { AudioHardwareCreateAggregateDevice(description, NonNull::from(&mut id)) };
        if status != 0 || id == 0 {
            return Err(format!("could not create the aggregate device (error {status})"));
        }
        Ok(Aggregate { id })
    }
}

impl Drop for Aggregate {
    fn drop(&mut self) {
        unsafe { AudioHardwareDestroyAggregateDevice(self.id) };
    }
}
