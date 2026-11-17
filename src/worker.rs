//! The single background thread that talks to GitHub so the UI never blocks.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use crate::app::{Request, Response};
use crate::github::PullRequestSource;

/// Starts the worker. It exits on its own once the request sender is dropped.
pub fn spawn(source: Arc<dyn PullRequestSource>) -> (Sender<Request>, Receiver<Response>) {
    let (request_tx, request_rx) = mpsc::channel::<Request>();
    let (response_tx, response_rx) = mpsc::channel::<Response>();
    thread::spawn(move || serve(source.as_ref(), request_rx, response_tx));
    (request_tx, response_rx)
}

fn serve(
    source: &dyn PullRequestSource,
    requests: Receiver<Request>,
    responses: Sender<Response>,
) {
    for request in requests {
        let response = match request {
            Request::LoadPullRequests => {
                Response::PullRequests(source.list_open().map_err(describe))
            }
            Request::LoadFiles { number } => Response::Files {
                number,
                result: source.changed_files(number).map_err(describe),
            },
        };
        if responses.send(response).is_err() {
            break;
        }
    }
}

fn describe(error: anyhow::Error) -> String {
    format!("{error:#}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use anyhow::bail;

    use crate::github::{
        ChangedFile, ChecksState, Conversation, FileStatus, PullRequest, ReviewDecision,
        ReviewSubmission,
    };

    struct FakeSource {
        fail: bool,
    }

    impl PullRequestSource for FakeSource {
        fn list_open(&self) -> anyhow::Result<Vec<PullRequest>> {
            if self.fail {
                bail!("boom");
            }
            Ok(vec![PullRequest {
                number: 1,
                title: "T".into(),
                author: "ann".into(),
                head_ref: "feat".into(),
                base_ref: "main".into(),
                head_sha: "sha".into(),
                draft: false,
                review_decision: ReviewDecision::None,
                checks: ChecksState::None,
                comment_count: 0,
            }])
        }

        fn changed_files(&self, number: u64) -> anyhow::Result<Vec<ChangedFile>> {
            Ok(vec![ChangedFile {
                filename: format!("f{number}.rs"),
                status: FileStatus::Added,
                additions: 1,
                deletions: 0,
                patch: None,
            }])
        }

        fn conversation(&self, _number: u64) -> anyhow::Result<Conversation> {
            unimplemented!()
        }

        fn post_comment(&self, _number: u64, _body: &str) -> anyhow::Result<()> {
            unimplemented!()
        }

        fn submit_review(&self, _number: u64, _review: &ReviewSubmission) -> anyhow::Result<()> {
            unimplemented!()
        }
    }

    const WAIT: Duration = Duration::from_secs(2);

    #[test]
    fn serves_pull_request_requests() {
        let (requests, responses) = spawn(Arc::new(FakeSource { fail: false }));
        requests.send(Request::LoadPullRequests).unwrap();
        match responses.recv_timeout(WAIT).unwrap() {
            Response::PullRequests(Ok(prs)) => assert_eq!(prs.len(), 1),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn serves_file_requests_with_number() {
        let (requests, responses) = spawn(Arc::new(FakeSource { fail: false }));
        requests.send(Request::LoadFiles { number: 7 }).unwrap();
        match responses.recv_timeout(WAIT).unwrap() {
            Response::Files { number: 7, result: Ok(files) } => assert_eq!(files[0].filename, "f7.rs"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn reports_errors_as_strings() {
        let (requests, responses) = spawn(Arc::new(FakeSource { fail: true }));
        requests.send(Request::LoadPullRequests).unwrap();
        match responses.recv_timeout(WAIT).unwrap() {
            Response::PullRequests(Err(message)) => assert!(message.contains("boom"), "{message}"),
            other => panic!("unexpected {other:?}"),
        }
    }
}
