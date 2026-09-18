//! Bounded pipes; no original or extracted text is written to temporary files.
use me_diagnostics::{Field as F, record};
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

#[cfg(any(target_os = "linux", test))]
pub(crate) fn run(
    command: &mut Command,
    input: &[u8],
    cancel: &AtomicBool,
    limit: usize,
) -> Result<(i32, Zeroizing<Vec<u8>>), String> {
    run_with_progress(command, input, cancel, limit, &mut |_, _| {})
}

pub(crate) fn run_with_progress(
    command: &mut Command,
    input: &[u8],
    cancel: &AtomicBool,
    limit: usize,
    progress: &mut impl FnMut(u32, u32),
) -> Result<(i32, Zeroizing<Vec<u8>>), String> {
    run_with_limits(
        command,
        input,
        cancel,
        limit,
        progress,
        Duration::from_secs(240),
        Duration::from_secs(3600),
    )
}

fn run_with_limits(
    command: &mut Command,
    input: &[u8],
    cancel: &AtomicBool,
    limit: usize,
    progress: &mut impl FnMut(u32, u32),
    idle_limit: Duration,
    total_limit: Duration,
) -> Result<(i32, Zeroizing<Vec<u8>>), String> {
    if cancel.load(Ordering::SeqCst) {
        return Err("File processing stopped.".into());
    }
    record(
        "text_helper.started",
        &[F::Count("input_bytes", input.len() as u64)],
    );
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            record(
                "text_helper.spawn_failed",
                &[F::Integer(
                    "os_error",
                    error.raw_os_error().unwrap_or(-1) as i64,
                )],
            );
            "Couldn't start text recognition. Check the installed document tools.".to_string()
        })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or("Text recognition is unavailable.")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Text recognition is unavailable.")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("Text recognition is unavailable.")?;
    let too_large = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let writer = scope.spawn(move || stdin.write_all(input));
        let (status_tx, status_rx) = std::sync::mpsc::sync_channel(16);
        let status_reader = scope.spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut discarding = false;
            loop {
                let mut line = Vec::new();
                match reader.by_ref().take(129).read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break,
                    _ => {}
                }
                let complete = line.ends_with(b"\n");
                if discarding || line.len() > 128 {
                    discarding = !complete;
                    continue;
                }
                if let Some(pair) = parse_progress(&line) {
                    let _ = status_tx.try_send(pair);
                }
            }
        });
        let large = &too_large;
        let reader = scope.spawn(move || {
            let mut bytes = Zeroizing::new(Vec::new());
            let result = stdout.take(limit as u64 + 1).read_to_end(&mut bytes);
            if bytes.len() > limit {
                large.store(true, Ordering::SeqCst);
            }
            result.map(|_| bytes)
        });
        let start = Instant::now();
        let mut activity = start;
        let mut last_page = None;
        let mut page_total = None;
        let status = loop {
            for (page, total) in status_rx.try_iter() {
                if page_total.is_none_or(|previous| previous == total)
                    && last_page.is_none_or(|previous| page > previous)
                {
                    last_page = Some(page);
                    page_total = Some(total);
                    activity = Instant::now();
                    progress(page, total);
                    record(
                        "text_helper.page",
                        &[
                            F::Count("page", page as u64),
                            F::Count("total", total as u64),
                        ],
                    );
                }
            }
            if cancel.load(Ordering::SeqCst)
                || too_large.load(Ordering::SeqCst)
                || activity.elapsed() > idle_limit
                || start.elapsed() > total_limit
            {
                let _ = child.kill();
                let _ = child.wait();
                break Err(if cancel.load(Ordering::SeqCst) {
                    "File processing stopped."
                } else {
                    "Text recognition exceeded the time or size limit."
                }
                .to_string());
            }
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status.code().unwrap_or(-1)),
                Ok(None) => std::thread::sleep(Duration::from_millis(25)),
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err("Texterkennung unterbrochen.".into());
                }
            }
        };
        record(
            "text_helper.finished",
            &[
                F::Integer("exit_code", status.as_ref().copied().unwrap_or(-1) as i64),
                F::Flag("cancelled", cancel.load(Ordering::SeqCst)),
                F::Flag("size_limit", too_large.load(Ordering::SeqCst)),
                F::Count("duration_ms", start.elapsed().as_millis() as u64),
            ],
        );
        status_reader
            .join()
            .map_err(|_| "Texterkennung unterbrochen.")?;
        // The last page may arrive immediately before process exit.
        for (page, total) in status_rx.try_iter() {
            if page_total.is_none_or(|old| old == total) && last_page.is_none_or(|old| page > old) {
                progress(page, total);
            }
        }
        let written = writer.join().map_err(|_| "Texterkennung unterbrochen.")?;
        let bytes = reader
            .join()
            .map_err(|_| "Texterkennung unterbrochen.")?
            .map_err(|_| "Couldn't read the text.")?;
        let status = status?;
        if bytes.len() > limit {
            return Err("Extracted text is too large.".into());
        }
        if status == 0 && written.is_err() {
            return Err("Couldn't read the entire file.".into());
        }
        Ok((status, bytes))
    })
}

fn parse_progress(line: &[u8]) -> Option<(u32, u32)> {
    let text = std::str::from_utf8(line).ok()?;
    let mut words = text.split_whitespace();
    if words.next()? != "ME_PAGE" {
        return None;
    }
    let page: u32 = words.next()?.parse().ok()?;
    let total: u32 = words.next()?.parse().ok()?;
    (words.next().is_none() && total > 0 && total <= me_core::MAX_DOCUMENT_PAGES && page <= total)
        .then_some((page, total))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancelling_a_running_child_does_not_wait_for_its_work() {
        let cancel = AtomicBool::new(false);
        let start = Instant::now();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(100));
                cancel.store(true, Ordering::SeqCst);
            });
            let result = run(
                Command::new("/bin/sh").args(["-c", "exec sleep 20"]),
                b"",
                &cancel,
                1024,
            );
            assert!(result.unwrap_err().contains("stopped"));
        });
        assert!(start.elapsed() < Duration::from_secs(3));
    }
    #[test]
    fn excess_output_is_stopped_and_never_returned_as_partial_success() {
        let result = run(
            Command::new("/bin/sh").args(["-c", "while :; do printf 'synthetic-data'; done"]),
            b"",
            &AtomicBool::new(false),
            64,
        );
        assert!(result.is_err());
    }
}

#[cfg(test)]
mod progress_tests {
    use super::*;
    #[test]
    fn verified_page_progress_extends_the_idle_deadline() {
        let mut pages = Vec::new();
        let mut command = Command::new("/usr/bin/python3");
        command.args(["-c", "import sys,time\nfor p in range(4):\n print('ME_PAGE',p,3,file=sys.stderr,flush=True)\n time.sleep(.08)\nprint('complete')"]);
        let (code, out) = run_with_limits(
            &mut command,
            b"",
            &AtomicBool::new(false),
            1024,
            &mut |p, t| pages.push((p, t)),
            Duration::from_millis(200),
            Duration::from_secs(3),
        )
        .unwrap();
        assert_eq!(code, 0);
        assert_eq!(&*out, b"complete\n");
        assert_eq!(pages.last(), Some(&(3, 3)));
    }
    #[test]
    fn garbage_or_repeated_status_cannot_keep_a_stalled_process_alive() {
        assert!(parse_progress(b"private data\n").is_none());
        assert!(parse_progress(b"ME_PAGE 2 1\n").is_none());
        assert!(parse_progress(b"ME_PAGE 0 501\n").is_none());
        let mut command = Command::new("/usr/bin/python3");
        command.args(["-c", "import sys,time\nfor p in range(20):\n print('ME_PAGE 0 2',file=sys.stderr,flush=True)\n time.sleep(.06)"]);
        let started = Instant::now();
        let result = run_with_limits(
            &mut command,
            b"",
            &AtomicBool::new(false),
            1024,
            &mut |_, _| {},
            Duration::from_millis(200),
            Duration::from_secs(3),
        );
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
