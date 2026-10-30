use super::{GATE_JOB, workflow_job_block};

const SERIALIZED_COMMAND: &str =
    "cargo test --test generated_output_downstream_gate -- --ignored --nocapture --test-threads=1";

pub(super) fn assert_serialized(tasks: &str, workflow: &str) {
    let tasks: toml::Value = toml::from_str(tasks).expect("parse mise task configuration");
    let commands = tasks["tasks"]["gate:generated-output"]["run"]
        .as_array()
        .expect("gate:generated-output must declare its commands");
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
    let block = workflow_job_block(workflow, GATE_JOB).expect("generated-output gate job must exist");
    assert!(
        block
            .lines()
            .any(|line| line.trim() == format!("run: {SERIALIZED_COMMAND}")),
        "the CI job must serialize its ignored generated-output invocation"
    );
}

#[cfg(test)]
mod tests {
    use super::{SERIALIZED_COMMAND, assert_serialized};

    fn task_config(command: &str) -> String {
        format!("[tasks.\"gate:generated-output\"]\nrun = ['{command}']\n")
    }

    fn workflow(command: &str) -> String {
        format!("jobs:\n  generated-output-gate:\n    steps:\n      - name: Run gate\n        run: {command}\n")
    }

    #[test]
    fn should_accept_the_single_maintained_task_and_ci_invocation() {
        let tasks = format!(
            "{}\n[tasks.unrelated]\nrun = ['{SERIALIZED_COMMAND}']\n",
            task_config(SERIALIZED_COMMAND)
        );
        assert_serialized(&tasks, &workflow(SERIALIZED_COMMAND));
    }

    #[test]
    #[should_panic(expected = "expected one ignored generated-output task invocation")]
    fn should_reject_the_task_when_ignored_lanes_are_removed() {
        let command = SERIALIZED_COMMAND.replace(" --ignored", "");
        assert_serialized(&task_config(&command), &workflow(SERIALIZED_COMMAND));
    }

    #[test]
    #[should_panic(expected = "the task must serialize every ignored lane")]
    fn should_reject_the_task_when_serialization_is_removed() {
        let command = SERIALIZED_COMMAND.replace(" --test-threads=1", "");
        assert_serialized(&task_config(&command), &workflow(SERIALIZED_COMMAND));
    }

    #[test]
    #[should_panic(expected = "the CI job must serialize its ignored generated-output invocation")]
    fn should_reject_ci_when_serialization_is_removed() {
        let command = SERIALIZED_COMMAND.replace(" --test-threads=1", "");
        assert_serialized(&task_config(SERIALIZED_COMMAND), &workflow(&command));
    }
}
