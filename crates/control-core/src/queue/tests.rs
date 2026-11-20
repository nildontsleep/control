use super::*;

fn tracks(q: &Queue) -> Vec<TrackId> {
    q.items().iter().map(|i| i.track).collect()
}

#[test]
fn play_next_prev_and_end() {
    let mut q = Queue::new();
    assert_eq!(q.play(&[1, 2, 3], 1, false), Step::Load { autoplay: true });
    assert_eq!(q.current().unwrap().track, 2);
    assert_eq!(q.next(Repeat::Off), Step::Load { autoplay: true });
    assert_eq!(q.current().unwrap().track, 3);
    assert_eq!(q.next(Repeat::Off), Step::Load { autoplay: false });
    assert_eq!(q.ended(Repeat::Off), Step::Stop);
    assert_eq!(q.ended(Repeat::All), Step::Load { autoplay: true });
    assert_eq!(q.current().unwrap().track, 1);
    assert_eq!(q.ended(Repeat::One), Step::Restart);
    assert_eq!(q.prev(10.0), Step::Restart);
    assert_eq!(q.prev(0.0), Step::Restart, "premier morceau");
    q.jump_to(2);
    assert_eq!(q.prev(1.0), Step::Load { autoplay: true });
    assert_eq!(q.current().unwrap().track, 2);
}

#[test]
fn shuffle_keeps_current_first_and_restores_order() {
    let mut q = Queue::new();
    let list: Vec<TrackId> = (1..=50).collect();
    q.play(&list, 10, true);
    assert_eq!(q.current().unwrap().track, 11);
    assert_eq!(q.index(), Some(0));
    let mut sorted = tracks(&q);
    sorted.sort();
    assert_eq!(sorted, list);
    q.set_shuffle(false);
    assert_eq!(tracks(&q), list);
    assert_eq!(q.current().unwrap().track, 11);
    q.set_shuffle(true);
    assert_eq!(q.index(), Some(0));
    assert_eq!(q.current().unwrap().track, 11);
}

#[test]
fn insertions_moves_and_removals() {
    let mut q = Queue::new();
    q.play(&[1, 2, 3], 0, false);
    q.play_next(&[9, 9], false);
    assert_eq!(tracks(&q), vec![1, 9, 9, 2, 3]);
    q.enqueue(&[7], false);
    assert_eq!(tracks(&q), vec![1, 9, 9, 2, 3, 7]);
    q.jump_to(3);
    q.move_item(0, 5);
    assert_eq!(tracks(&q), vec![9, 9, 2, 3, 7, 1]);
    assert_eq!(q.current().unwrap().track, 2);
    q.move_item(5, 0);
    assert_eq!(q.current().unwrap().track, 2);
    assert_eq!(q.index(), Some(3));
    q.move_item(3, 0);
    assert_eq!(q.index(), Some(3), "la courante ne bouge pas");
    q.remove_at(0);
    assert_eq!(q.current().unwrap().track, 2);
    q.remove_at(q.index().unwrap());
    assert_eq!(q.current().unwrap().track, 2, "la courante ne se retire pas");
    q.clear_upcoming();
    assert_eq!(q.upcoming().len(), 0);
    // L'ordre d'origine suit : sans aléatoire on retrouve ce qui reste.
    q.set_shuffle(false);
    assert_eq!(q.current().unwrap().track, 2);
}

#[test]
fn prune_and_session() {
    let mut q = Queue::new();
    q.play(&[1, 2, 3, 4], 1, false);
    assert_eq!(q.prune(|t| t != 3), Step::Nothing);
    assert_eq!(tracks(&q), vec![1, 2, 4]);
    assert_eq!(q.prune(|t| t != 2), Step::Load { autoplay: false });
    assert_eq!(q.current().unwrap().track, 4);
    let s = q.session(42.7).unwrap();
    assert_eq!(s, Session { queue: vec![1, 4], index: 1, position: 42.0 });
    let json = serde_json::to_value(&s).unwrap();
    assert_eq!(json, serde_json::json!({"queue": [1, 4], "index": 1, "position": 42.0}));
    let mut r = Queue::new();
    assert_eq!(r.restore(&s, |t| t != 1), Some(42.0));
    assert_eq!(r.current().unwrap().track, 4);
    assert_eq!(q.prune(|_| false), Step::Stop);
    assert!(q.is_empty());
}

#[test]
fn peek_next_respects_repeat() {
    let mut q = Queue::new();
    q.play(&[1, 2], 1, false);
    assert_eq!(q.peek_next(Repeat::Off), None);
    assert_eq!(q.peek_next(Repeat::All).unwrap().track, 1);
    assert_eq!(q.peek_next(Repeat::One), None);
    let uid = q.peek_next(Repeat::All).unwrap().uid;
    assert!(q.advance_to(uid));
    assert_eq!(q.index(), Some(0));
}

#[test]
fn listen_counts_once_after_threshold() {
    let mut l = Listen::default();
    l.reset(Some(5));
    let mut counted = 0;
    let mut t = 0.0;
    while t < 40.0 {
        if l.tick(t, 200.0) {
            counted += 1;
        }
        t += 0.5;
    }
    assert_eq!(counted, 1);
    assert!(l.listened >= 30.0);
    // Un saut en avant ne compte pas comme écouté.
    let mut l = Listen::default();
    l.reset(Some(5));
    l.tick(0.0, 20.0);
    assert!(!l.tick(15.0, 20.0));
    assert!(l.listened < 1.0);
    // Morceau court : la moitié suffit.
    let mut l = Listen::default();
    l.reset(Some(5));
    let mut t = 0.0;
    let mut hit = false;
    while t <= 6.0 {
        hit |= l.tick(t, 10.0);
        t += 0.25;
    }
    assert!(hit);
}
