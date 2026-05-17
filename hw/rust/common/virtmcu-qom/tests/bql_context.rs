use virtmcu_qom::device::BqlContext;

#[test]
fn test_bql_context_creation() {
    let _ctx = unsafe { BqlContext::new_unchecked() };
}

static_assertions::assert_not_impl_any!(BqlContext: Send, Sync);

/// ```compile_fail
/// use virtmcu_qom::sync::BqlGuarded;
/// let guarded = BqlGuarded::new(42);
/// let _guard = guarded.get();
/// ```
pub struct BqlGuardedGetCompileFail;

/// ```compile_fail
/// use virtmcu_qom::sync::BqlGuarded;
/// let guarded = BqlGuarded::new(42);
/// let mut _guard = guarded.get_mut();
/// ```
pub struct BqlGuardedGetMutCompileFail;
