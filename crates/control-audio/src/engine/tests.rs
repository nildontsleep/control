//! Le moteur complet, avec ses threads, sur la sortie muette (aucune carte son nécessaire).
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use super::*;
use crate::testutil::{sine, write_wav};

fn null_engine() -> Engine {
    Engine::new(EngineConfig { output: OutputMode::Null, ..Default::default() }).unwrap()
}

fn wait_for(rx: &Receiver<EngineEvent>, want: &EngineEvent, timeout: Duration) -> Vec<EngineEvent> {
    let deadline = Instant::now() + timeout;
    let mut seen = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(e) => {
                let hit = &e == want;
                seen.push(e);
                if hit {
                    return seen;
                }
            }
            Err(_) => break,
        }
    }
    panic!("{want:?} pas reçu ; reçus : {seen:?}");
}

#[test]
fn engine_is_shareable() {
    fn check<T: Clone + Send + Sync>() {}
    check::<Engine>();
}

#[test]
fn null_output_plays_in_real_time_and_chains_tracks() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.wav");
    let b = dir.path().join("b.wav");
    write_wav(&a, 44_100, 1, &sine(440.0, 44_100, 1.0, 0.3));
    write_wav(&b, 22_050, 1, &sine(660.0, 22_050, 0.6, 0.3));
    let engine = null_engine();
    assert_eq!(engine.output_device(), None);
    let rx = engine.events();
    engine.load(1, &a, 0.0);
    engine.preload_next(2, &b);
    wait_for(&rx, &EngineEvent::Started { id: 1 }, Duration::from_secs(3));
    let t0 = Instant::now();
    std::thread::sleep(Duration::from_millis(400));
    let s = engine.status();
    let elapsed = t0.elapsed().as_secs_f64();
    assert!(s.playing && s.track == Some(1) && s.next == Some(2), "{s:?}");
    assert!((s.position - elapsed).abs() < 0.2, "position {} après {elapsed:.2} s", s.position);
    wait_for(&rx, &EngineEvent::Advanced { from: 1, to: 2 }, Duration::from_secs(3));
    wait_for(&rx, &EngineEvent::Ended { id: 2 }, Duration::from_secs(3));
    let total = t0.elapsed().as_secs_f64();
    assert!(total > 1.2 && total < 2.2, "{total:.2} s pour 1,6 s de son");
    std::thread::sleep(Duration::from_millis(50));
    assert!(!engine.status().playing);
}

#[test]
fn pause_is_immediate_and_resume_continues() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.wav");
    write_wav(&a, 48_000, 1, &sine(440.0, 48_000, 5.0, 0.3));
    let engine = null_engine();
    let rx = engine.events();
    engine.load(1, &a, 0.0);
    wait_for(&rx, &EngineEvent::Started { id: 1 }, Duration::from_secs(3));
    std::thread::sleep(Duration::from_millis(500));
    engine.pause();
    std::thread::sleep(Duration::from_millis(60));
    let p1 = engine.position();
    assert!(!engine.is_playing());
    // La position entendue, pas celle du rendu (qui a ~150 ms d'avance).
    assert!(p1 > 0.3 && p1 < 0.75, "{p1}");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(engine.position(), p1);
    engine.play();
    std::thread::sleep(Duration::from_millis(400));
    let p2 = engine.position();
    assert!(p2 > p1 + 0.2 && p2 < p1 + 0.6, "{p1} → {p2}");
    engine.seek(4.0);
    std::thread::sleep(Duration::from_millis(100));
    assert!((engine.position() - 4.0).abs() < 0.2);
    engine.stop();
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(engine.current(), None);
}

#[test]
fn default_output_falls_back_gracefully() {
    // Avec ou sans carte son : le moteur démarre, sait dire où il joue, et s'arrête proprement.
    let engine = Engine::new(EngineConfig::default()).unwrap();
    assert!(engine.sample_rate() >= 8_000);
    let clone = engine.clone();
    std::thread::spawn(move || clone.set_volume(0.5)).join().unwrap();
    assert_eq!(engine.status().track, None);
    drop(engine);
}
