use super::*;

fn pipeline() -> (EditingProxies, Receiver<Job>, mpsc::Sender<Completion>) {
    let (tx, jobs) = mpsc::sync_channel(1);
    let (done, rx) = mpsc::channel();
    (
        EditingProxies {
            tx: Some(tx),
            rx,
            worker: None,
            session: 0,
            next_id: 0,
            sources: Vec::new(),
            ready: HashMap::new(),
            failed: HashSet::new(),
            active: None,
        },
        jobs,
        done,
    )
}

fn ready(job: Job) -> Completion {
    Completion {
        id: job.id,
        session: job.session,
        path: job.path,
        result: Ok(EditProxy {
            path: "cache/proxy.mp4".into(),
            source_width: 1920,
            source_height: 1080,
            width: 1280,
            height: 720,
        }),
    }
}

#[test]
fn switching_project_cancels_old_work_and_cannot_install_its_result() {
    let (mut queue, jobs, done) = pipeline();
    queue.set_sources(1, vec!["a.mp4".into()]);
    queue.pump(None);
    let old = jobs.recv().unwrap();
    queue.set_sources(2, vec!["a.mp4".into()]);
    assert!(old.cancelled.load(Ordering::Acquire));
    queue.pump(None);
    let current = jobs.recv().unwrap();
    done.send(ready(old)).unwrap();
    assert!(queue.poll().is_empty());
    assert!(queue.ready.is_empty());
    done.send(ready(current)).unwrap();
    assert_eq!(
        queue.poll(),
        vec![("a.mp4".into(), PathBuf::from("cache/proxy.mp4"))]
    );
}

#[test]
fn visible_source_is_prioritized_and_repeated_frames_do_not_queue_duplicate_work() {
    let (mut queue, jobs, done) = pipeline();
    queue.set_sources(1, vec!["a.mp4".into(), "b.mp4".into()]);
    queue.pump(Some("b.mp4"));
    let first = jobs.recv().unwrap();
    assert_eq!(first.path, "b.mp4");
    for _ in 0..60 {
        queue.pump(Some("b.mp4"));
    }
    assert!(matches!(jobs.try_recv(), Err(mpsc::TryRecvError::Empty)));
    done.send(ready(first)).unwrap();
    queue.poll();
    queue.pump(Some("b.mp4"));
    assert_eq!(jobs.recv().unwrap().path, "a.mp4");
}

#[test]
fn failed_proxy_falls_back_without_reencoding_forever_and_removed_job_is_cancelled() {
    let (mut queue, jobs, done) = pipeline();
    queue.set_sources(1, vec!["a.mp4".into()]);
    queue.pump(None);
    let job = jobs.recv().unwrap();
    done.send(Completion {
        id: job.id,
        session: job.session,
        path: job.path,
        result: Err(MediaError::Invalid("not video".into())),
    })
    .unwrap();
    assert!(queue.poll().is_empty());
    queue.pump(None);
    assert!(matches!(jobs.try_recv(), Err(mpsc::TryRecvError::Empty)));
    assert!(queue.summary().unwrap().contains("1 项使用原素材"));
    queue.set_sources(1, vec!["b.mp4".into()]);
    queue.pump(None);
    let removed = jobs.recv().unwrap();
    queue.set_sources(1, vec![]);
    assert!(removed.cancelled.load(Ordering::Acquire));
    done.send(ready(removed)).unwrap();
    assert!(queue.poll().is_empty());
    assert!(queue.summary().is_none());
}
