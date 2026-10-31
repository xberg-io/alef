use super::{GATE_JOB, workflow_job_block};

const UMBRELLA_TASK: &str = "gate:generated-output";
const LANES_TASK: &str = "gate:generated-output:lanes";
const SERIALIZED_COMMAND: &str =
    "cargo test --test generated_output_downstream_gate -- --ignored --nocapture --test-threads=1";

pub(super) fn assert_serialized(tasks: &str, workflow: &str) {
    let tasks: toml::Value = toml::from_str(tasks).expect("parse mise task configuration");
    let commands = tasks["tasks"][LANES_TASK]["run"]
        .as_array()
        .expect("gate:generated-output:lanes must declare its commands");
    let ignored: Vec<&str> = commands
        .iter()
        .filter_map(toml::Value::as_str)
        .filter(|command| command.contains("generated_output_downstream_gate") && command.contains("--ignored"))
        .collect();
    assert_eq!(
        ignored.len(),
        1,
        "expected one ignored generated-output task invocation"
    );
    assert_eq!(
        ignored[0], SERIALIZED_COMMAND,
        "the task must serialize every ignored lane"
    );
    let umbrella_runs_lanes = tasks["tasks"][UMBRELLA_TASK]["run"].as_array().is_some_and(|steps| {
        steps
            .iter()
            .any(|step| step.get("task").and_then(toml::Value::as_str) == Some(LANES_TASK))
    });
    assert!(
        umbrella_runs_lanes,
        "the local gate:generated-output task must run the lanes task CI runs"
    );
    let block = workflow_job_block(workflow, GATE_JOB).expect("generated-output gate job must exist");
    assert!(
        block
            .lines()
            .any(|line| line.trim() == format!("run: mise run {LANES_TASK}")),
        "the CI job must run the serialized generated-output lanes task"
    );
}

#[cfg(test)]
mod tests {
    use super::{LANES_TASK, SERIALIZED_COMMAND, UMBRELLA_TASK, assert_serialized};

    fn task_config(command: &str) -> String {
        format!(
            "[tasks.\"{UMBRELLA_TASK}\"]\nrun = [{{ task = \"{LANES_TASK}\" }}]\n\
             [tasks.\"{LANES_TASK}\"]\nrun = ['{command}']\n"
        )
    }

    fn workflow(command: &str) -> String {
        format!("jobs:\n  generated-output-gate:\n    steps:\n      - name: Run gate\n        run: {command}\n")
    }

    fn ci_lanes_step() -> String {
        format!("mise run {LANES_TASK}")
    }

    #[test]
    fn should_accept_the_single_maintained_task_and_ci_invocation() {
        let tasks = format!(
            "{}\n[tasks.unrelated]\nrun = ['{SERIALIZED_COMMAND}']\n",
            task_config(SERIALIZED_COMMAND)
        );
        assert_serialized(&tasks, &workflow(&ci_lanes_step()));
    }

    #[test]
    #[should_panic(expected = "expected one ignored generated-output task invocation")]
    fn should_reject_the_task_when_ignored_lanes_are_removed() {
        let command = SERIALIZED_COMMAND.replace(" --ignored", "");
        assert_serialized(&task_config(&command), &workflow(&ci_lanes_step()));
    }

    #[test]
    #[should_panic(expected = "the task must serialize every ignored lane")]
    fn should_reject_the_task_when_serialization_is_removed() {
        let command = SERIALIZED_COMMAND.replace(" --test-threads=1", "");
        assert_serialized(&task_config(&command), &workflow(&ci_lanes_step()));
    }

    #[test]
    #[should_panic(expected = "the local gate:generated-output task must run the lanes task CI runs")]
    fn should_reject_a_local_task_that_skips_the_lanes() {
        let tasks = task_config(SERIALIZED_COMMAND).replace(&format!("{{ task = \"{LANES_TASK}\" }}"), "'true'");
        assert_serialized(&tasks, &workflow(&ci_lanes_step()));
    }

    #[test]
    #[should_panic(expected = "the CI job must run the serialized generated-output lanes task")]
    fn should_reject_ci_that_bypasses_the_lanes_task() {
        assert_serialized(&task_config(SERIALIZED_COMMAND), &workflow(SERIALIZED_COMMAND));
    }
}
