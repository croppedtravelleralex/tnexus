//! Web dispatch pin 对齐纯函数（对齐 Go `web_pool_pins.go` + `imagine_slots.go`）。
//!
//! `imageDispatchPinTargetIds`、`ImagineSlotRegistryIDs`、`TicketReadyAccountIDs` 为
//! 纯函数；`SyncImageDispatchPins`（Service 接线 + 仓库 ReplaceRouteBound）属 G3-P5。

/// 返回 grok-imagine-image 应 pin 的账号集合。
///
/// 默认与整个 dispatch 对齐（票只影响选号排序，不缩小可选集合，避免 503）。
/// 配置了 imagine slot account ids 时，pin 与 SlotRegistry 对齐。
pub fn image_dispatch_pin_target_ids(
    configured_slot_ids: &[i64],
    dispatch_ids: &[i64],
) -> Vec<i64> {
    if configured_slot_ids.is_empty() {
        return dispatch_ids.to_vec();
    }
    imagine_slot_registry_ids(configured_slot_ids, dispatch_ids)
}

/// BE-024 SlotRegistry 视图。
///
/// `configured` 为空时默认整个 dispatch 池都在槽位内（向后兼容，不收窄 pin）。
pub fn imagine_slot_registry_ids(configured: &[i64], dispatch: &[i64]) -> Vec<i64> {
    if configured.is_empty() {
        return dispatch.to_vec();
    }
    intersect_sorted(configured, dispatch)
}

/// TicketReady = slotRegistry ∩ dispatch ∩ pin ∩ 有可用票。
///
/// `ticket_counts` 为空时视为无票约束（返回 slot∩dispatch∩pin）。
pub fn ticket_ready_account_ids(
    slot_registry: &[i64],
    dispatch: &[i64],
    pin_ids: &[i64],
    ticket_counts: &std::collections::HashMap<i64, i64>,
) -> Vec<i64> {
    let mut base = intersect_sorted(dispatch, slot_registry);
    if !pin_ids.is_empty() {
        base = intersect_sorted(&base, pin_ids);
    }
    if ticket_counts.is_empty() {
        return base;
    }
    base.into_iter()
        .filter(|id| ticket_counts.get(id).copied().unwrap_or(0) > 0)
        .collect()
}

/// 两个升序整数切片的交集（对齐 Go `intersectSortedUint64`）。
pub fn intersect_sorted(a: &[i64], b: &[i64]) -> Vec<i64> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(a.len());
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                out.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out
}
