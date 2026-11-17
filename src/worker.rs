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
            Request::LoadConversation { number } => Response::Conversation {
                number,
                result: source.conversation(number).map_err(describe),
            },
            Request::PostComment { number, body } => Response::CommentPosted {
                number,
                result: source.post_comment(number, &body).map_err(describe),
            },
            Request::SubmitReview { number, review } => Response::ReviewSubmitted {
                number,
                result: source.submit_review(number, &review).map_err(describe),
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
    use std::sync::Mutex;
    use std::time::Duration;

    use anyhow::bail;

    use crate::github::{
        ChangedFile, ChecksState, Conversation, FileStatus, PullRequest, ReviewDecision,
        ReviewEvent, ReviewSubmission,
    };

    struct FakeSource {
        fail: bool,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl FakeSource {
        fn new(fail: bool) -> FakeSource {
            FakeSource { fail, calls: Arc::new(Mutex::new(Vec::new())) }
        }
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
            Ok(Conversation::default())
        }

        fn post_comment(&self, number: u64, body: &str) -> anyhow::Result<()> {
            self.calls.lock().unwrap().push(format!("post_comment {number} {body}"));
            Ok(())
        }

        fn submit_review(&self, number: u64, review: &ReviewSubmission) -> anyhow::Result<()> {
            self.calls.lock().unwrap().push(format!("submit_review {number} {:?}", review.event));
            Ok(())
        }
    }

    const WAIT: Duration = Duration::from_secs(2);

    #[test]
    fn serves_pull_request_requests() {
        let (requests, responses) = spawn(Arc::new(FakeSource::new(false)));
        requests.send(Request::LoadPullRequests).unwrap();
        match responses.recv_timeout(WAIT).unwrap() {
            Response::PullRequests(Ok(prs)) => assert_eq!(prs.len(), 1),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn serves_file_requests_with_number() {
        let (requests, responses) = spawn(Arc::new(FakeSource::new(false)));
        requests.send(Request::LoadFiles { number: 7 }).unwrap();
        match responses.recv_timeout(WAIT).unwrap() {
            Response::Files { number: 7, result: Ok(files) } => assert_eq!(files[0].filename, "f7.rs"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn reports_errors_as_strings() {
        let (requests, responses) = spawn(Arc::new(FakeSource::new(true)));
        requests.send(Request::LoadPullRequests).unwrap();
        match responses.recv_timeout(WAIT).unwrap() {
            Response::PullRequests(Err(message)) => assert!(message.contains("boom"), "{message}"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn serves_conversation_requests() {
        let (requests, responses) = spawn(Arc::new(FakeSource::new(false)));
        requests.send(Request::LoadConversation { number: 7 }).unwrap();
        match responses.recv_timeout(WAIT).unwrap() {
            Response::Conversation { number: 7, result: Ok(_) } => {}
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn serves_comment_and_review_requests() {
        let source = FakeSource::new(false);
        let calls = Arc::clone(&source.calls);
        let (requests, responses) = spawn(Arc::new(source));
        requests.send(Request::PostComment { number: 7, body: "hi".into() }).unwrap();
        let review = ReviewSubmission {
            event: ReviewEvent::Approve,
            body: String::new(),
            commit_sha: "s".into(),
            comments: vec![],
        };
        requests.send(Request::SubmitReview { number: 7, review }).unwrap();
        assert!(matches!(
            responses.recv_timeout(WAIT).unwrap(),
            Response::CommentPosted { number: 7, result: Ok(()) }
        ));
        assert!(matches!(
            responses.recv_timeout(WAIT).unwrap(),
            Response::ReviewSubmitted { number: 7, result: Ok(()) }
        ));
        assert_eq!(*calls.lock().unwrap(), ["post_comment 7 hi", "submit_review 7 Approve"]);
    }
}
