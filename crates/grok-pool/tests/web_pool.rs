//! G3-P2 Web 图池选择与 dispatch pin 集成测试（迁移 Go `web_pool_test.go` +
//! `web_pool_pins_test.go` + `imagine_slots.go` 行为）。

use chrono::{DateTime, Utc};

use grok_domain::{ModelState, ModelStatus, QuotaSource, QuotaWindow};
use grok_pool::pins::image_dispatch_pin_target_ids;
use grok_pool::web_pool::{
    image_dispatch_admissible, image_pool_eligible, select_web_pool_ids, WebPoolCandidate,
};

fn fresh_window(total: i64, remaining: i64, synced: DateTime<Utc>) -> QuotaWindow {
    QuotaWindow {
        account_id: 1,
        mode: "imagine".into(),
        remaining,
        total,
        synced_at: Some(synced),
        source: QuotaSource::Upstream,
        updated_at: synced,
        ..Default::default()
    }
}

// === TestSelectWebPoolIDsRespectsCapAndOrdering ===

#[test]
fn select_web_pool_ids_respects_cap_and_ordering() {
    let candidates = vec![
        WebPoolCandidate { id: 1, priority: 1, fast_rem: 10, ..Default::default() },
        WebPoolCandidate { id: 2, priority: 5, fast_rem: 1, ..Default::default() },
        WebPoolCandidate { id: 3, priority: 5, fast_rem: 9, ..Default::default() },
        WebPoolCandidate { id: 4, priority: 0, fast_rem: 30, ..Default::default() },
    ];
    let ids = select_web_pool_ids(
        &candidates,
        2,
        |c| c.fast_rem > 0,
        |a, b| {
            if a.priority != b.priority {
                a.priority > b.priority
            } else if a.fast_rem != b.fast_rem {
                a.fast_rem > b.fast_rem
            } else {
                a.id < b.id
            }
        },
    );
    assert_eq!(ids, vec![3, 2], "want [3 2] by priority then remaining");
}

// === TestImagePoolEligibilityRequiresFreshPositiveImagineQuota ===

#[test]
fn image_pool_eligibility_cases() {
    let now = Utc::now();
    let synced = now - chrono::Duration::minutes(5);
    let findex = |total, remaining| fresh_window(total, remaining, synced);

    let cases: Vec<(&str, WebPoolCandidate, bool)> = vec![
        (
            "zero over zero is blocked",
            WebPoolCandidate { enabled: true, active: true, imagine_window: Some(findex(0, 0)), ..Default::default() },
            false,
        ),
        (
            "zero over zero quota available awaits probe",
            WebPoolCandidate {
                enabled: true, active: true, imagine_window: Some(findex(0, 0)),
                model_state: Some(ModelState { status: ModelStatus::QuotaAvailable, ..Default::default() }),
                ..Default::default()
            },
            true,
        ),
        (
            "zero over zero recent lite success",
            WebPoolCandidate {
                enabled: true, active: true, imagine_window: Some(findex(0, 0)),
                model_state: Some(ModelState {
                    status: ModelStatus::Available,
                    last_success_at: Some(now - chrono::Duration::minutes(10)),
                    ..Default::default()
                }),
                ..Default::default()
            },
            true,
        ),
        (
            "known zero is exhausted",
            WebPoolCandidate { enabled: true, active: true, imagine_window: Some(findex(10, 0)), ..Default::default() },
            false,
        ),
        (
            "known positive is available",
            WebPoolCandidate { enabled: true, active: true, imagine_window: Some(findex(10, 3)), ..Default::default() },
            true,
        ),
        (
            "stale positive with unknown state is blocked",
            WebPoolCandidate {
                enabled: true, active: true,
                imagine_window: Some(fresh_window(10, 3, now - chrono::Duration::hours(2))),
                model_state: Some(ModelState { status: ModelStatus::Unknown, ..Default::default() }),
                ..Default::default()
            },
            false,
        ),
        (
            "signature failure is unavailable",
            WebPoolCandidate {
                enabled: true, active: true,
                model_state: Some(ModelState { status: ModelStatus::SignatureFailed, ..Default::default() }),
                ..Default::default()
            },
            false,
        ),
        (
            "old quota exhaustion cleared by positive refresh",
            WebPoolCandidate {
                enabled: true, active: true, imagine_blocked: true, imagine_window: Some(findex(10, 3)),
                model_state: Some(ModelState { status: ModelStatus::QuotaExhausted, ..Default::default() }),
                ..Default::default()
            },
            true,
        ),
        (
            "active soft stop is unavailable",
            WebPoolCandidate {
                enabled: true, active: true,
                model_state: Some(ModelState {
                    status: ModelStatus::SoftStop,
                    cooldown_until: Some(now + chrono::Duration::minutes(1)),
                    ..Default::default()
                }),
                ..Default::default()
            },
            false,
        ),
        (
            "dispatch requires available state",
            WebPoolCandidate {
                enabled: true, active: true, imagine_window: Some(findex(10, 3)),
                model_state: Some(ModelState { status: ModelStatus::Available, ..Default::default() }),
                ..Default::default()
            },
            true,
        ),
    ];

    for (name, candidate, want) in cases {
        let got = image_pool_eligible(&candidate, now);
        assert_eq!(got, want, "case: {name}");
    }
}

// === TestImageDispatchAdmissibleStricterThanEligible ===

#[test]
fn image_dispatch_admissible_stricter_than_eligible() {
    let now = Utc::now();
    let synced = now - chrono::Duration::minutes(5);
    let fresh = fresh_window(10, 3, synced);

    // exhausted + blocked: eligible allows refreshed exhaustion, dispatch rejects.
    let exhausted = WebPoolCandidate {
        enabled: true, active: true, imagine_blocked: true, imagine_window: Some(fresh.clone()),
        model_state: Some(ModelState { status: ModelStatus::QuotaExhausted, ..Default::default() }),
        ..Default::default()
    };
    assert!(image_pool_eligible(&exhausted, now), "eligible should allow refreshed exhaustion");
    assert!(!image_dispatch_admissible(&exhausted, now), "dispatch must reject quota_exhausted");

    // available + fresh quota: dispatch accepts.
    let available = WebPoolCandidate {
        enabled: true, active: true, imagine_window: Some(fresh.clone()),
        model_state: Some(ModelState { status: ModelStatus::Available, ..Default::default() }),
        ..Default::default()
    };
    assert!(image_dispatch_admissible(&available, now));

    // unknown gate + recent lite success: dispatch accepts.
    let unknown_recent = WebPoolCandidate {
        enabled: true, active: true,
        imagine_window: Some(fresh_window(0, 0, synced)),
        model_state: Some(ModelState {
            status: ModelStatus::Available,
            last_success_at: Some(now - chrono::Duration::minutes(10)),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(image_dispatch_admissible(&unknown_recent, now), "dispatch should accept unknown gate w/ recent success");
}

// === pins：imageDispatchPinTargetIDs + SlotRegistry ===

#[test]
fn pin_target_keeps_full_dispatch_by_default() {
    let dispatch = vec![86, 87, 227, 250];
    let got = image_dispatch_pin_target_ids(&[], &dispatch);
    assert_eq!(got, dispatch, "want full dispatch when no slots configured");
}

#[test]
fn pin_target_uses_slot_registry_when_configured() {
    let dispatch = vec![86, 87, 227, 250];
    let got = image_dispatch_pin_target_ids(&[87, 250, 999], &dispatch);
    assert_eq!(got, vec![87, 250]);
}

#[test]
fn pin_target_falls_back_without_tickets() {
    let dispatch = vec![86, 87, 227];
    let got = image_dispatch_pin_target_ids(&[], &dispatch);
    assert_eq!(got, dispatch);
}
