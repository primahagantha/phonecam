//! Windows WinMM (`waveOut`) low-latency 16-bit mono PCM audio playback.
//!
//! Phone audio arrives as raw 16-bit little-endian mono PCM chunks over
//! `ChannelRole::Audio`. By default PhoneCam runs in monitor-only mode (driving
//! only the UI VU meter to prevent speaker/mic feedback). When the user selects
//! `"default"` or a named Windows playback device (such as VB-Audio Virtual
//! Cable's `"CABLE Input"`), [`WaveOutPlayer`] streams those chunks through
//! WinMM using pre-allocated ping-pong / ring buffers with zero per-packet heap
//! allocations on the audio hot path.

/// Number of pre-allocated WinMM buffer slots (~170 ms max ring at 48 kHz / 2048 B).
pub const RING_SLOTS: usize = 8;

/// Byte capacity of each pre-allocated PCM slot (matches `run_audio` read buffer).
pub const SLOT_CAPACITY_BYTES: usize = 4096;

/// Returns `true` if `name` looks like a virtual audio cable / loopback input
/// device (e.g. VB-Audio Virtual Cable `"CABLE Input"`).
pub fn is_virtual_audio_cable(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("cable input")
        || lower.contains("vb-audio")
        || lower.contains("virtual audio")
        || lower.contains("voicemeeter")
}

/// Normalizes a configured audio output setting.
///
/// `None`, empty strings, and `"monitor_only"` all normalize to `None`
/// (VU meter only, no speaker/device playback).
pub fn normalize_output_device(device: Option<&str>) -> Option<String> {
    device
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "monitor_only")
        .map(ToString::to_string)
}

/// Deduplicates and sorts playback devices so virtual audio cables appear first
/// while preserving relative driver enumeration order.
pub fn prioritize_devices(devices: Vec<String>) -> Vec<String> {
    let mut virtual_cables = Vec::new();
    let mut standard = Vec::new();

    for raw in devices {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let name = trimmed.to_string();
        if virtual_cables.contains(&name) || standard.contains(&name) {
            continue;
        }
        if is_virtual_audio_cable(&name) {
            virtual_cables.push(name);
        } else {
            standard.push(name);
        }
    }

    virtual_cables.extend(standard);
    virtual_cables
}

#[cfg(windows)]
mod platform {
    use std::ptr::{addr_of, null_mut};
    use std::sync::atomic::{fence, Ordering};
    use windows::core::PSTR;
    use windows::Win32::Media::Audio::{
        waveOutClose, waveOutGetDevCapsW, waveOutGetNumDevs, waveOutOpen, waveOutPrepareHeader,
        waveOutReset, waveOutUnprepareHeader, waveOutWrite, CALLBACK_NULL, HWAVEOUT, WAVEFORMATEX,
        WAVEHDR, WAVEOUTCAPSW, WAVE_FORMAT_PCM, WAVE_MAPPER, WHDR_DONE,
    };

    use super::{prioritize_devices, RING_SLOTS, SLOT_CAPACITY_BYTES};

    const MMSYSERR_NOERROR: u32 = 0;

    fn decode_wide_name(raw: &[u16]) -> String {
        let len = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
        String::from_utf16_lossy(&raw[..len]).trim().to_string()
    }

    fn raw_windows_devices() -> Vec<(u32, String)> {
        // SAFETY: `waveOutGetNumDevs` takes no pointers and returns the count of
        // installed waveform-audio output devices from WinMM.
        let count = unsafe { waveOutGetNumDevs() };
        let mut out = Vec::with_capacity(count as usize);
        let caps_size = std::mem::size_of::<WAVEOUTCAPSW>() as u32;

        for idx in 0..count {
            let mut caps = WAVEOUTCAPSW::default();
            // SAFETY: `caps` is a valid, zero-initialized `WAVEOUTCAPSW` buffer of size `caps_size`.
            let res = unsafe { waveOutGetDevCapsW(idx as usize, &mut caps, caps_size) };
            if res == MMSYSERR_NOERROR {
                // Copy `szPname` out of the `#[repr(C, packed(1))]` struct by value
                // to avoid creating an unaligned slice reference.
                let raw_name = caps.szPname;
                let name = decode_wide_name(&raw_name);
                if !name.is_empty() {
                    out.push((idx, name));
                }
            }
        }
        out
    }

    /// Enumerates active Windows audio playback devices, prioritizing virtual
    /// audio cables (e.g. `"CABLE Input"`) at the front of the list.
    pub fn list_output_devices() -> Vec<String> {
        let names = raw_windows_devices()
            .into_iter()
            .map(|(_, name)| name)
            .collect();
        prioritize_devices(names)
    }

    struct AudioSlot {
        hdr: Box<WAVEHDR>,
        data: Box<[u8]>,
        prepared: bool,
    }

    impl AudioSlot {
        fn new(capacity: usize) -> Self {
            Self {
                hdr: Box::new(WAVEHDR::default()),
                data: vec![0u8; capacity].into_boxed_slice(),
                prepared: false,
            }
        }

        fn is_done(&self) -> bool {
            if !self.prepared {
                return true;
            }
            fence(Ordering::Acquire);
            // SAFETY: `self.hdr` points to a valid heap-allocated `WAVEHDR`.
            // `read_unaligned` is required because `WAVEHDR` is `#[repr(C, packed(1))]`.
            let flags = unsafe { addr_of!(self.hdr.dwFlags).read_unaligned() };
            (flags & WHDR_DONE) != 0
        }
    }

    /// WinMM `waveOut` player with pre-allocated ring buffers.
    pub struct WaveOutPlayer {
        hwo: HWAVEOUT,
        slots: Vec<AudioSlot>,
        cursor: usize,
        sample_rate: u32,
        device_name: String,
    }

    impl WaveOutPlayer {
        /// Opens a Windows playback device (`"default"` uses `WAVE_MAPPER`, any
        /// other string matches a device name returned by [`list_output_devices`]).
        pub fn open(device: &str, sample_rate: u32) -> Result<Self, String> {
            let rate = if sample_rate == 0 {
                48_000
            } else {
                sample_rate
            };
            let device_id = if device.eq_ignore_ascii_case("default") {
                WAVE_MAPPER
            } else {
                raw_windows_devices()
                    .into_iter()
                    .find(|(_, name)| name == device)
                    .map(|(id, _)| id)
                    .ok_or_else(|| format!("audio output device not found: {device}"))?
            };

            let wfx = WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_PCM as u16,
                nChannels: 1,
                nSamplesPerSec: rate,
                nAvgBytesPerSec: rate * 2,
                nBlockAlign: 2,
                wBitsPerSample: 16,
                cbSize: 0,
            };

            let mut hwo = HWAVEOUT::default();
            // SAFETY: `hwo` and `wfx` are valid stack-allocated structs that live for
            // the duration of `waveOutOpen`; `CALLBACK_NULL` indicates polling via
            // `WAVEHDR.dwFlags & WHDR_DONE` without callbacks.
            let open_res =
                unsafe { waveOutOpen(Some(&mut hwo), device_id, &wfx, None, None, CALLBACK_NULL) };
            if open_res != MMSYSERR_NOERROR || hwo.is_invalid() {
                return Err(format!(
                    "waveOutOpen failed for '{device}' at {rate} Hz (code {open_res})"
                ));
            }

            let mut slots = Vec::with_capacity(RING_SLOTS);
            for _ in 0..RING_SLOTS {
                slots.push(AudioSlot::new(SLOT_CAPACITY_BYTES));
            }

            Ok(Self {
                hwo,
                slots,
                cursor: 0,
                sample_rate: rate,
                device_name: device.to_string(),
            })
        }

        pub fn sample_rate(&self) -> u32 {
            self.sample_rate
        }

        pub fn device_name(&self) -> &str {
            &self.device_name
        }

        /// Writes 16-bit mono PCM bytes to the WinMM output ring without allocating.
        pub fn write_pcm(&mut self, pcm: &[u8]) -> Result<(), String> {
            let aligned_len = pcm.len() & !1;
            if aligned_len == 0 {
                return Ok(());
            }

            for chunk in pcm[..aligned_len].chunks(SLOT_CAPACITY_BYTES) {
                self.write_chunk(chunk)?;
            }
            Ok(())
        }

        fn write_chunk(&mut self, chunk: &[u8]) -> Result<(), String> {
            let slot = &mut self.slots[self.cursor];
            if slot.prepared && !slot.is_done() {
                // Brief bounded wait (up to 4 ms) so jitter bursts don't immediately
                // drop frames, while ensuring the audio socket reader never stalls.
                let mut ready = false;
                for _ in 0..4 {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    if slot.is_done() {
                        ready = true;
                        break;
                    }
                }
                if !ready {
                    tracing::debug!("audio_out ring full; dropping {} byte chunk", chunk.len());
                    return Ok(());
                }
            }

            let hdr_size = std::mem::size_of::<WAVEHDR>() as u32;
            if slot.prepared {
                let hdr_ptr: *mut WAVEHDR = &mut *slot.hdr;
                // SAFETY: `self.hwo` is an open `HWAVEOUT` handle, `hdr_ptr` points to the
                // heap-allocated `WAVEHDR` previously prepared with `waveOutPrepareHeader`,
                // and `WHDR_DONE` was observed set.
                let unprep = unsafe { waveOutUnprepareHeader(self.hwo, hdr_ptr, hdr_size) };
                if unprep != MMSYSERR_NOERROR {
                    tracing::warn!("waveOutUnprepareHeader returned {unprep}");
                }
                slot.prepared = false;
            }

            slot.data[..chunk.len()].copy_from_slice(chunk);
            slot.hdr.lpData = PSTR(slot.data.as_mut_ptr());
            slot.hdr.dwBufferLength = chunk.len() as u32;
            slot.hdr.dwBytesRecorded = 0;
            slot.hdr.dwUser = 0;
            slot.hdr.dwFlags = 0;
            slot.hdr.dwLoops = 0;
            slot.hdr.lpNext = null_mut();
            slot.hdr.reserved = 0;

            let hdr_ptr: *mut WAVEHDR = &mut *slot.hdr;
            // SAFETY: `self.hwo` is open; `hdr_ptr` and its `lpData` point to boxed heap
            // allocations whose addresses remain stable until `Drop` / `waveOutUnprepareHeader`,
            // and `dwBufferLength <= slot.data.len()`.
            let prep_res = unsafe { waveOutPrepareHeader(self.hwo, hdr_ptr, hdr_size) };
            if prep_res != MMSYSERR_NOERROR {
                return Err(format!("waveOutPrepareHeader failed (code {prep_res})"));
            }
            slot.prepared = true;

            // SAFETY: `hdr_ptr` is prepared and valid for `chunk.len()` bytes of `slot.data`;
            // WinMM retains read access until `WHDR_DONE` is set or `waveOutReset` completes.
            let write_res = unsafe { waveOutWrite(self.hwo, hdr_ptr, hdr_size) };
            if write_res != MMSYSERR_NOERROR {
                return Err(format!("waveOutWrite failed (code {write_res})"));
            }

            self.cursor = (self.cursor + 1) % self.slots.len();
            Ok(())
        }
    }

    impl Drop for WaveOutPlayer {
        fn drop(&mut self) {
            if !self.hwo.is_invalid() {
                // SAFETY: `self.hwo` is an open WinMM output handle; `waveOutReset` stops
                // playback and marks all queued headers `WHDR_DONE` so they can be
                // safely unprepared before freeing `slots`.
                unsafe {
                    let _ = waveOutReset(self.hwo);
                }

                let hdr_size = std::mem::size_of::<WAVEHDR>() as u32;
                for slot in &mut self.slots {
                    if slot.prepared {
                        let hdr_ptr: *mut WAVEHDR = &mut *slot.hdr;
                        // SAFETY: `waveOutReset` has completed and `hdr_ptr` was prepared on `self.hwo`.
                        unsafe {
                            let _ = waveOutUnprepareHeader(self.hwo, hdr_ptr, hdr_size);
                        }
                        slot.prepared = false;
                    }
                }

                // SAFETY: All prepared headers have been reset and unprepared; closing `self.hwo`
                // releases the WinMM device handle exactly once.
                unsafe {
                    let _ = waveOutClose(self.hwo);
                }
                self.hwo = HWAVEOUT::default();
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    /// Non-Windows stub: returns no Windows playback devices.
    pub fn list_output_devices() -> Vec<String> {
        Vec::new()
    }

    /// Non-Windows stub for `WaveOutPlayer`.
    pub struct WaveOutPlayer {
        sample_rate: u32,
        device_name: String,
    }

    impl WaveOutPlayer {
        pub fn open(device: &str, sample_rate: u32) -> Result<Self, String> {
            let _ = (device, sample_rate);
            Err("Windows waveOut audio playback is only supported on Windows".into())
        }

        pub fn sample_rate(&self) -> u32 {
            self.sample_rate
        }

        pub fn device_name(&self) -> &str {
            &self.device_name
        }

        pub fn write_pcm(&mut self, _pcm: &[u8]) -> Result<(), String> {
            Ok(())
        }
    }
}

pub use platform::{list_output_devices, WaveOutPlayer};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_audio_cable_detection_matches_vb_cable_and_similar() {
        assert!(is_virtual_audio_cable(
            "CABLE Input (VB-Audio Virtual Cable)"
        ));
        assert!(is_virtual_audio_cable("VB-Audio Point"));
        assert!(is_virtual_audio_cable("Line 1 (Virtual Audio Cable)"));
        assert!(is_virtual_audio_cable(
            "VoiceMeeter Input (VB-Audio VoiceMeeter VAIO)"
        ));
        assert!(!is_virtual_audio_cable("Speakers (Realtek(R) Audio)"));
        assert!(!is_virtual_audio_cable("Headphones (USB Audio)"));
    }

    #[test]
    fn prioritize_devices_places_virtual_cables_first_and_dedups() {
        let input = vec![
            "Speakers (Realtek(R) Audio)".to_string(),
            "  ".to_string(),
            "CABLE Input (VB-Audio Virtual Cable)".to_string(),
            "Speakers (Realtek(R) Audio)".to_string(),
            "Headphones (Sony)".to_string(),
            "VoiceMeeter Input".to_string(),
        ];
        let prioritized = prioritize_devices(input);
        assert_eq!(
            prioritized,
            vec![
                "CABLE Input (VB-Audio Virtual Cable)",
                "VoiceMeeter Input",
                "Speakers (Realtek(R) Audio)",
                "Headphones (Sony)",
            ]
        );
    }

    #[test]
    fn normalize_output_device_treats_monitor_only_and_empty_as_none() {
        assert_eq!(normalize_output_device(None), None);
        assert_eq!(normalize_output_device(Some("")), None);
        assert_eq!(normalize_output_device(Some("   ")), None);
        assert_eq!(normalize_output_device(Some("monitor_only")), None);
        assert_eq!(
            normalize_output_device(Some("default")),
            Some("default".into())
        );
        assert_eq!(
            normalize_output_device(Some(" CABLE Input (VB-Audio) ")),
            Some("CABLE Input (VB-Audio)".into())
        );
    }

    #[test]
    fn list_output_devices_enumerates_without_panicking() {
        let devices = list_output_devices();
        // On Windows CI / dev machines with or without audio hardware, this must succeed safely.
        for dev in &devices {
            assert!(!dev.trim().is_empty());
        }
    }

    #[test]
    fn opening_nonexistent_device_returns_clean_error() {
        let res = WaveOutPlayer::open("__NonExistent_PhoneCam_Audio_Device__", 48_000);
        assert!(res.is_err());
    }
}
