//! `--bench`: measures the performance targets of PLAN.md on this machine.
//!
//! It opens the real overlay several times (the screen dims briefly), draws
//! frames, exports and encodes a PNG in memory, and prints the timings and
//! the memory use. Captures stay in memory: nothing is written to disk.
//!
//! ```text
//! simple-screen-grabber.exe --bench | Out-Host
//! ```

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use windows::core::{w, Result};
use windows::Win32::Foundation::{GENERIC_WRITE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Console::{
    AttachConsole, GetConsoleMode, GetStdHandle, WriteConsoleW, ATTACH_PARENT_PROCESS,
    CONSOLE_MODE, STD_OUTPUT_HANDLE,
};
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::Win32::UI::WindowsAndMessaging::DefWindowProcW;

use crate::capture;
use crate::export::png;
use crate::fluent::Theme;
use crate::geometry::Rect;
use crate::overlay::Overlay;
use crate::win;

const ITERATIONS: usize = 10;
const FRAMES: usize = 30;

/// Working set and private memory, in MB.
#[derive(Clone, Copy, Default)]
struct Memory {
    working_set: f64,
    private: f64,
}

fn memory() -> Memory {
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    // SAFETY: the structure is initialized with its size.
    unsafe {
        let _ = GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        );
    }
    Memory {
        working_set: counters.WorkingSetSize as f64 / 1_048_576.0,
        private: counters.PrivateUsage as f64 / 1_048_576.0,
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Minimum, median and maximum in milliseconds.
fn stats(values: &mut [Duration]) -> (f64, f64, f64) {
    values.sort();
    match values {
        [] => (0.0, 0.0, 0.0),
        _ => (
            ms(values[0]),
            ms(values[values.len() / 2]),
            ms(values[values.len() - 1]),
        ),
    }
}

extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: default handling for this thread's window.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Runs the benchmark and prints the report.
pub fn run() -> Result<()> {
    let mut out = String::new();
    let start_memory = memory();
    // SAFETY: COM for this thread, before any COM call.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };

    let started = Instant::now();
    let mut overlay = Overlay::new(Some(window_proc), Theme::load())?;
    let startup = started.elapsed();
    let idle = memory();
    win::trim_working_set();
    let idle_trimmed = memory();

    // GDI copy of the screen, with and without CAPTUREBLT.
    let mut with_layered = Vec::new();
    let mut without_layered = Vec::new();
    for _ in 0..ITERATIONS {
        let t = Instant::now();
        drop(capture::capture_screen_with(true, false)?);
        with_layered.push(t.elapsed());
        let t = Instant::now();
        drop(capture::capture_screen_with(false, false)?);
        without_layered.push(t.elapsed());
    }

    // Warm-up, not counted.
    overlay.open()?;
    overlay.close();

    // First feedback: the screen dims before the copy starts.
    let mut feedback = Vec::new();
    let mut dimmer = true;
    for _ in 0..ITERATIONS {
        let t = Instant::now();
        dimmer = overlay.bench_dim(true);
        // SAFETY: waits for the next desktop composition.
        unsafe { DwmFlush()? };
        feedback.push(t.elapsed());
        overlay.bench_dim(false);
        // SAFETY: as above, so the next run starts undimmed.
        unsafe { DwmFlush()? };
    }

    let mut warm = Vec::new();
    let mut cold = Vec::new();
    let (mut captures, mut uploads, mut first_frames) = (Vec::new(), Vec::new(), Vec::new());
    let mut frames = Vec::new();
    let (mut crops, mut renders, mut encodes) = (Vec::new(), Vec::new(), Vec::new());
    let mut open_memory = Memory::default();
    let mut closed_memory = Memory::default();
    let mut png_size = (0usize, 0u32, 0u32);
    for i in 0..ITERATIONS {
        // Every other run starts from a trimmed working set, as after the
        // app sat idle.
        let trimmed = i % 2 == 1;
        if trimmed {
            win::trim_working_set();
        }
        let t = Instant::now();
        overlay.open()?;
        // SAFETY: waits for the next desktop composition.
        unsafe { DwmFlush()? };
        let visible = t.elapsed();
        if trimmed {
            cold.push(visible);
        } else {
            warm.push(visible);
        }
        let timings = overlay.last_open;
        captures.push(timings.capture);
        uploads.push(timings.upload);
        first_frames.push(timings.first_frame);

        let screen = overlay.screen_size();
        let sel = Rect::new(
            screen.0 / 5,
            screen.1 / 5,
            screen.0 * 4 / 5,
            screen.1 * 4 / 5,
        );
        for f in 0..FRAMES {
            let grow = (f as i32 % 10) * 4;
            let rect = Rect::new(
                sel.left - grow,
                sel.top - grow,
                sel.right + grow,
                sel.bottom + grow,
            );
            frames.push(overlay.bench_frame(rect)?);
        }

        let t = Instant::now();
        let (image, _) = overlay.export().ok_or_else(windows::core::Error::empty)??;
        crops.push(t.elapsed());
        let t = Instant::now();
        let encoded = png::to_bytes(&image)?;
        encodes.push(t.elapsed());
        png_size = (encoded.len(), image.width, image.height);
        drop((image, encoded));

        overlay.bench_annotate(sel);
        let t = Instant::now();
        let (image, _) = overlay.export().ok_or_else(windows::core::Error::empty)??;
        renders.push(t.elapsed());
        drop(image);

        open_memory = memory();
        overlay.close();
        closed_memory = memory();
    }
    win::trim_working_set();
    let closed_trimmed = memory();

    let exe_size = std::env::current_exe()
        .and_then(std::fs::metadata)
        .map(|m| m.len())
        .unwrap_or(0);
    let (sw, sh) = overlay.screen_size();
    let line = |out: &mut String, name: &str, values: &mut Vec<Duration>| {
        let (min, median, max) = stats(values);
        let _ = writeln!(
            out,
            "  {name:<34} {median:>7.1} ms   (min {min:.1}, max {max:.1})"
        );
    };
    let _ = writeln!(out, "Simple Screen Grabber benchmark");
    let _ = writeln!(out, "Virtual screen: {sw} x {sh} px, {ITERATIONS} runs");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Startup (devices, window, swap chain): {:.1} ms",
        ms(startup)
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "Shortcut to the screen dimming (first feedback)");
    if dimmer {
        line(&mut out, "dimmed on screen", &mut feedback);
    } else {
        let _ = writeln!(out, "  not available on this Windows version");
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "Shortcut to the frozen overlay visible");
    line(&mut out, "warm", &mut warm);
    line(&mut out, "after trimming the working set", &mut cold);
    line(&mut out, "  screen copy (GDI)", &mut captures);
    line(&mut out, "  upload", &mut uploads);
    line(&mut out, "  first frame", &mut first_frames);
    let _ = writeln!(out);
    let _ = writeln!(out, "Screen copy");
    line(
        &mut out,
        "with CAPTUREBLT (layered windows)",
        &mut with_layered,
    );
    line(&mut out, "without CAPTUREBLT", &mut without_layered);
    let _ = writeln!(out);
    let _ = writeln!(out, "Frame drawing (input to frame, before vsync)");
    line(&mut out, "frame", &mut frames);
    let _ = writeln!(out);
    let _ = writeln!(out, "Export of a {} x {} selection", png_size.1, png_size.2);
    line(&mut out, "copy of the pixels", &mut crops);
    line(&mut out, "drawn with an annotation", &mut renders);
    line(&mut out, "PNG encoding", &mut encodes);
    let raw = f64::from(png_size.1) * f64::from(png_size.2) * 3.0;
    let _ = writeln!(
        out,
        "  PNG size: {:.1} MB ({:.0}% of the raw RGB pixels)",
        png_size.0 as f64 / 1_048_576.0,
        png_size.0 as f64 / raw * 100.0
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "Memory (working set / private, MB)");
    let mem = |out: &mut String, name: &str, m: Memory| {
        let _ = writeln!(
            out,
            "  {name:<34} {:>7.1} / {:.1}",
            m.working_set, m.private
        );
    };
    mem(&mut out, "process start", start_memory);
    mem(&mut out, "idle", idle);
    mem(&mut out, "idle, working set trimmed", idle_trimmed);
    mem(&mut out, "overlay open", open_memory);
    mem(&mut out, "after close", closed_memory);
    mem(&mut out, "after close, trimmed", closed_trimmed);
    let _ = writeln!(out);
    let _ = writeln!(out, "Executable: {:.0} KB", exe_size as f64 / 1024.0);
    print(&out);
    Ok(())
}

/// Writes to the console the app was started from, or to a redirected
/// standard output. The app is a GUI program, so it has no console of its
/// own.
fn print(text: &str) {
    // SAFETY: handles from the system, written with buffers that outlive
    // the calls.
    unsafe {
        let redirected = GetStdHandle(STD_OUTPUT_HANDLE)
            .ok()
            .filter(|h| !h.is_invalid() && !h.0.is_null());
        if let Some(handle) = redirected {
            let mut mode = CONSOLE_MODE::default();
            if GetConsoleMode(handle, &mut mode).is_ok() {
                let wide: Vec<u16> = text.encode_utf16().collect();
                let _ = WriteConsoleW(handle, &wide, None, None);
            } else {
                let _ = WriteFile(handle, Some(text.as_bytes()), None, None);
            }
            return;
        }
        if AttachConsole(ATTACH_PARENT_PROCESS).is_ok() {
            if let Ok(console) = CreateFileW(
                w!("CONOUT$"),
                GENERIC_WRITE.0,
                FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            ) {
                let wide: Vec<u16> = text.encode_utf16().collect();
                let _ = WriteConsoleW(console, &wide, None, None);
                let _ = windows::Win32::Foundation::CloseHandle(console);
            }
        }
    }
}
