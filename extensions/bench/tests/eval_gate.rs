use agent_brain_bench::eval::{assert_ci_gate, run_ci_eval, RECALL_AT_3_THRESHOLD};
use agent_brain_bench::fixture::new_isolated_engine;

#[test]
fn eval_ci_meets_recall_gate() {
    let (engine, _dir) = new_isolated_engine().unwrap();
    let report = run_ci_eval(&engine).unwrap();
    assert!(report.recall_at_3 >= RECALL_AT_3_THRESHOLD, "{report:?}");
    assert_ci_gate(&report).unwrap();
}
