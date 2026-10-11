//! Retry preparation only, before any chat message is sent.
use std::{future::Future, time::Duration};

pub(super) async fn prepare_with_retry<T, F, Fut, N, W, Wait>(
    mut prepare: F,
    mut limited: N,
    mut wait: W,
) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, String>>,
    N: FnMut(Duration),
    W: FnMut(Duration) -> Wait,
    Wait: Future<Output = ()>,
{
    for attempt in 0..3 {
        match prepare().await {
            Err(error) if super::sticker_pace::rate_limited(&error) => {
                let delay = Duration::from_secs(30 << attempt);
                limited(delay);
                if attempt == 2 {
                    return Err("WhatsApp is temporarily limiting sticker uploads. Please wait two minutes before trying again.".to_owned());
                }
                wait(delay).await;
            }
            result => return result,
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque};

    #[tokio::test]
    async fn rate_limits_wait_then_prepare_successfully() {
        let mut results = VecDeque::from([
            Err("received a server error response: code=429 text=rate-overlimit".to_owned()),
            Err("Upload failed with status: 429".to_owned()),
            Ok(42),
        ]);
        let notices = RefCell::new(Vec::new());
        let waits = RefCell::new(Vec::new());
        let result = prepare_with_retry(
            || std::future::ready(results.pop_front().unwrap()),
            |delay| notices.borrow_mut().push(delay.as_secs()),
            |delay| {
                waits.borrow_mut().push(delay.as_secs());
                std::future::ready(())
            },
        )
        .await;
        assert_eq!(result, Ok(42));
        assert!(results.is_empty());
        assert_eq!(*notices.borrow(), [30, 60]);
        assert_eq!(*waits.borrow(), [30, 60]);
    }

    #[tokio::test]
    async fn persistent_rate_limit_stops_after_three_attempts() {
        let mut attempts = 0;
        let waits = RefCell::new(Vec::new());
        let result = prepare_with_retry::<(), _, _, _, _, _>(
            || {
                attempts += 1;
                std::future::ready(Err("code=429".to_owned()))
            },
            |_| {},
            |delay| {
                waits.borrow_mut().push(delay.as_secs());
                std::future::ready(())
            },
        )
        .await;
        assert!(result.unwrap_err().contains("two minutes"));
        assert_eq!(attempts, 3);
        assert_eq!(*waits.borrow(), [30, 60]);
    }

    #[tokio::test]
    async fn other_errors_are_not_retried() {
        for error in ["Not connected to WhatsApp", "invalid WebP", "status: 403"] {
            let result = prepare_with_retry::<(), _, _, _, _, _>(
                || std::future::ready(Err(error.to_owned())),
                |_| panic!("not a rate limit"),
                |_| async { panic!("must not retry") },
            )
            .await;
            assert_eq!(result, Err(error.to_owned()));
        }
    }
}
