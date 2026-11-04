use octo_net::mux::{INITIAL_WINDOW, MAX_DATA, Mux};
use std::time::Duration;
use tokio::sync::mpsc;

fn pair() -> (Mux, Mux) {
    let (ab_tx, ab_rx) = mpsc::channel(64);
    let (ba_tx, ba_rx) = mpsc::channel(64);
    (Mux::new(ab_tx, ba_rx, true), Mux::new(ba_tx, ab_rx, false))
}

async fn read_all(s: &mut octo_net::mux::Stream) -> Vec<u8> {
    let mut v = Vec::new();
    while let Some(c) = s.read().await.unwrap() {
        v.extend(c);
    }
    v
}

#[tokio::test]
async fn interleaved_streams_keep_their_bytes() {
    let (a, mut b) = pair();
    let mut tasks = Vec::new();
    let mut expected = Vec::new();
    for i in 0..8u8 {
        let s = a.open().await.unwrap();
        let data: Vec<u8> = (0..200_000u32).map(|n| (n as u8).wrapping_add(i)).collect();
        expected.push(data.clone());
        tasks.push(tokio::spawn(async move {
            for c in data.chunks(7_000) {
                s.write(c).await.unwrap();
                tokio::task::yield_now().await;
            }
            s.finish().await.unwrap();
        }));
    }
    let mut readers = Vec::new();
    for _ in 0..8 {
        let mut s = b.accept().await.unwrap();
        readers.push(tokio::spawn(async move { read_all(&mut s).await }));
    }
    let mut got: Vec<Vec<u8>> = Vec::new();
    for r in readers {
        got.push(r.await.unwrap());
    }
    for t in tasks {
        t.await.unwrap();
    }
    // Accept order == open order, so stream k carries pattern k.
    assert_eq!(got, expected);
}

#[tokio::test]
async fn bidirectional_request_response() {
    let (a, mut b) = pair();
    let srv = tokio::spawn(async move {
        let mut s = b.accept().await.unwrap();
        let req = read_all(&mut s).await;
        s.write(&[req.as_slice(), b"-resp"].concat()).await.unwrap();
        s.finish().await.unwrap();
        b
    });
    let mut s = a.open().await.unwrap();
    s.write(b"req").await.unwrap();
    s.finish().await.unwrap();
    assert_eq!(read_all(&mut s).await, b"req-resp");
    srv.await.unwrap();
}

#[tokio::test]
async fn backpressure_blocks_writer_until_reader_consumes() {
    let (a, mut b) = pair();
    let s = a.open().await.unwrap();
    let mut r = b.accept().await.unwrap();
    let total = INITIAL_WINDOW as usize * 3;
    let data = vec![7u8; total];
    // Nobody reads: the writer must stall at the window.
    let blocked = tokio::time::timeout(Duration::from_millis(300), s.write(&data)).await;
    assert!(
        blocked.is_err(),
        "write must block when window is exhausted"
    );
    // A new writer on the same stream after cancellation is fine; now drain and write again.
    let drain = tokio::spawn(async move {
        let mut n = 0usize;
        while n < INITIAL_WINDOW as usize {
            n += r.read().await.unwrap().unwrap().len();
        }
        (r, n)
    });
    let (mut r, n) = drain.await.unwrap();
    assert!(n >= INITIAL_WINDOW as usize);
    // Window replenished: a fresh write of one window completes while reader keeps reading.
    let reader = tokio::spawn(async move {
        let mut got = 0usize;
        while got < INITIAL_WINDOW as usize + 1000 {
            got += r.read().await.unwrap().unwrap().len();
        }
    });
    tokio::time::timeout(
        Duration::from_secs(5),
        s.write(&vec![1u8; INITIAL_WINDOW as usize]),
    )
    .await
    .expect("write completes after reader consumed")
    .unwrap();
    reader.abort();
}

#[tokio::test]
async fn one_slow_stream_does_not_block_another() {
    let (a, mut b) = pair();
    let slow = a.open().await.unwrap();
    let fast = a.open().await.unwrap();
    let _slow_r = b.accept().await.unwrap(); // never read
    let mut fast_r = b.accept().await.unwrap();
    let w = tokio::spawn(async move {
        let _ = tokio::time::timeout(
            Duration::from_millis(200),
            slow.write(&vec![0; INITIAL_WINDOW as usize * 2]),
        )
        .await;
    });
    fast.write(b"still flows").await.unwrap();
    fast.finish().await.unwrap();
    assert_eq!(read_all(&mut fast_r).await, b"still flows");
    w.await.unwrap();
}

#[tokio::test]
async fn peer_exceeding_window_gets_reset() {
    let (out_tx, mut out_rx) = mpsc::channel(64);
    let (in_tx, in_rx) = mpsc::channel(64);
    let mut m = Mux::new(out_tx, in_rx, false);
    let frame = |id: u32, kind: u8, body: &[u8]| {
        let mut f = id.to_be_bytes().to_vec();
        f.push(kind);
        f.extend(body);
        f
    };
    in_tx.send(frame(1, 0, &[])).await.unwrap(); // open
    let mut s = m.accept().await.unwrap();
    let chunk = vec![0u8; MAX_DATA];
    for _ in 0..(INITIAL_WINDOW as usize / MAX_DATA + 2) {
        in_tx.send(frame(1, 1, &chunk)).await.unwrap();
    }
    // Reader eventually sees a reset (after any already-buffered data).
    let r = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match s.read().await {
                Err(_) => return true,
                Ok(None) => return false,
                Ok(Some(_)) => continue,
            }
        }
    })
    .await
    .unwrap();
    assert!(r);
    // And we told the peer.
    let mut saw_reset = false;
    while let Ok(f) = out_rx.try_recv() {
        if f[4] == 3 {
            saw_reset = true;
        }
    }
    assert!(saw_reset);
}

#[tokio::test]
async fn malformed_and_oversize_frames_are_ignored_or_reset() {
    let (out_tx, _out_rx) = mpsc::channel(64);
    let (in_tx, in_rx) = mpsc::channel(64);
    let mut m = Mux::new(out_tx, in_rx, false);
    in_tx.send(vec![1, 2]).await.unwrap(); // too short
    in_tx.send(vec![0, 0, 0, 1, 99]).await.unwrap(); // unknown kind
    in_tx.send(vec![0, 0, 0, 1, 0]).await.unwrap(); // valid open
    let mut s = m.accept().await.unwrap();
    let mut big = vec![0, 0, 0, 1, 1];
    big.extend(vec![0u8; MAX_DATA + 1]);
    in_tx.send(big).await.unwrap();
    assert!(s.read().await.is_err());
}

#[tokio::test]
async fn reset_aborts_peer_writer_and_reader() {
    let (a, mut b) = pair();
    let s = a.open().await.unwrap();
    let mut r = b.accept().await.unwrap();
    s.reset().await;
    assert!(r.read().await.is_err());
}

#[tokio::test]
async fn idle_stream_with_stalled_peer_is_reset_on_both_sides() {
    let (ab_tx, ab_rx) = mpsc::channel(64);
    let (ba_tx, ba_rx) = mpsc::channel(64);
    // Only A has a short idle timeout; B's own watchdog (default 60 s) must not be the cause.
    let a = Mux::with_idle(ab_tx, ba_rx, true, Duration::from_millis(300));
    let mut b = Mux::new(ba_tx, ab_rx, false);
    let s = a.open().await.unwrap();
    let mut peer = b.accept().await.unwrap();
    // B never reads: A's write exceeds the window, gets no credit, and must fail by itself.
    let big = vec![7u8; INITIAL_WINDOW as usize + 10_000];
    let r = tokio::time::timeout(Duration::from_secs(5), s.write(&big)).await;
    assert!(r.expect("write must not hang").is_err());
    // B's side sees the reset (after draining what was already delivered).
    let end = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if peer.read().await.is_err() {
                return;
            }
        }
    })
    .await;
    assert!(end.is_ok(), "peer stream must be reset");
}

#[tokio::test]
async fn active_stream_is_not_idle_reset() {
    let (ab_tx, ab_rx) = mpsc::channel(64);
    let (ba_tx, ba_rx) = mpsc::channel(64);
    let a = Mux::with_idle(ab_tx, ba_rx, true, Duration::from_millis(300));
    let mut b = Mux::with_idle(ba_tx, ab_rx, false, Duration::from_millis(300));
    let s = a.open().await.unwrap();
    let mut peer = b.accept().await.unwrap();
    for _ in 0..8 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        s.write(b"tick").await.unwrap();
        assert_eq!(peer.read().await.unwrap().unwrap(), b"tick");
    }
}
