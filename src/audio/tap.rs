use super::devices;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2_core_audio::{
    AudioHardwareCreateProcessTap, AudioHardwareDestroyProcessTap, AudioObjectID,
    CATapDescription, CATapMuteBehavior,
};
use objc2_foundation::{NSArray, NSInteger, NSNumber, NSString, ns_string};

/// A stereo mix of what plays on one output device, except this process. While the tap is
/// being read the tapped processes are muted on that device, so only our processed copy is
/// heard there. Audio that apps send to other devices is neither tapped nor muted.
pub struct Tap {
    pub id: AudioObjectID,
    pub uid: Retained<NSString>,
}

impl Tap {
    /// Taps output stream `stream`, counted from 0, of the device `device_uid`. If macOS will
    /// not tap that device on its own, taps everything the system plays instead.
    pub fn new(device_uid: &NSString, stream: usize) -> Result<Tap, String> {
        // Without this exclusion our own output would feed back into the tap.
        let own = devices::own_process_object()
            .ok_or("could not find this process in Core Audio (needed to avoid feedback)")?;
        let excluded = NSArray::from_retained_slice(&[NSNumber::new_u32(own)]);

        let device = unsafe {
            let d = CATapDescription::initExcludingProcesses_andDeviceUID_withStream(
                CATapDescription::alloc(),
                &excluded,
                device_uid,
                stream as NSInteger,
            );
            // A stereo mix of the stream, however many channels the device gives it.
            d.setMixdown(true);
            d.setMono(false);
            d
        };
        create(&device).or_else(|error| {
            eprintln!("{error}; tapping all system audio instead");
            let global = unsafe {
                CATapDescription::initStereoGlobalTapButExcludeProcesses(
                    CATapDescription::alloc(),
                    &excluded,
                )
            };
            create(&global)
        })
    }
}

/// Creates the tap described, private to EQ, muting what it taps while it is read.
fn create(description: &CATapDescription) -> Result<Tap, String> {
    unsafe {
        description.setName(ns_string!("EQ"));
        description.setPrivate(true);
        description.setMuteBehavior(CATapMuteBehavior::MutedWhenTapped);
    }
    let uid = unsafe { description.UUID().UUIDString() };

    let mut id: AudioObjectID = 0;
    let status = unsafe { AudioHardwareCreateProcessTap(Some(description), &mut id) };
    if status != 0 || id == 0 {
        return Err(format!(
            "could not create the system audio tap (error {})",
            devices::os_status(status)
        ));
    }
    Ok(Tap { id, uid })
}

impl Drop for Tap {
    fn drop(&mut self) {
        unsafe { AudioHardwareDestroyProcessTap(self.id) };
    }
}
