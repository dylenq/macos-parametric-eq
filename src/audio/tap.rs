use super::devices;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2_core_audio::{
    AudioHardwareCreateProcessTap, AudioHardwareDestroyProcessTap, AudioObjectID,
    CATapDescription, CATapMuteBehavior,
};
use objc2_foundation::{NSArray, NSNumber, NSString, ns_string};

/// A stereo mixdown of everything the system plays except this process. While the tap is
/// being read the tapped processes are muted at the hardware, so only our processed copy
/// is heard.
pub struct Tap {
    pub id: AudioObjectID,
    pub uid: Retained<NSString>,
}

impl Tap {
    pub fn new() -> Result<Tap, String> {
        // Without this exclusion our own output would feed back into the tap.
        let own = devices::own_process_object()
            .ok_or("could not find this process in Core Audio (needed to avoid feedback)")?;
        let excluded = NSArray::from_retained_slice(&[NSNumber::new_u32(own)]);

        let description = unsafe {
            let d = CATapDescription::initStereoGlobalTapButExcludeProcesses(
                CATapDescription::alloc(),
                &excluded,
            );
            d.setName(ns_string!("EQ"));
            d.setPrivate(true);
            d.setMuteBehavior(CATapMuteBehavior::MutedWhenTapped);
            d
        };
        let uid = unsafe { description.UUID().UUIDString() };

        let mut id: AudioObjectID = 0;
        let status = unsafe { AudioHardwareCreateProcessTap(Some(&description), &mut id) };
        if status != 0 || id == 0 {
            return Err(format!("could not create the system audio tap (error {status})"));
        }
        Ok(Tap { id, uid })
    }
}

impl Drop for Tap {
    fn drop(&mut self) {
        unsafe { AudioHardwareDestroyProcessTap(self.id) };
    }
}
