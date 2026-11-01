use std::process::Command;

use crate::snippets::validators::process::run_command_streams;

pub(crate) fn assert_resource_edges(script: &str) {
    let blocks: Vec<_> = script
        .match_indices("tasks.matching {")
        .filter_map(|(start, _)| {
            let rest = &script[start..];
            let end = rest.find("\n}")? + 2;
            let block = &rest[..end];
            (block.contains("UnitTestJavaRes") && block.contains("dependsOn(\"copyHostJni\")")).then_some(block)
        })
        .collect();
    assert_eq!(blocks.len(), 1, "exactly one actual emitted resource dependency rule");
    let root = tempfile::tempdir().expect("isolated real Gradle project");
    let fixture = format!(
        r#"val consumers = listOf("processDebugUnitTestJavaRes", "processReleaseUnitTestJavaRes",
    "compressDebugUnitTestJavaRes", "compressReleaseUnitTestJavaRes")
val unrelated = listOf("compressDebugJavaRes", "compileDebugUnitTestKotlin")
tasks.register("copyHostJni")
(consumers + unrelated).forEach {{ tasks.register(it) }}

{}

tasks.register("verifyGeneratedResourceEdges") {{
    doLast {{
        val enabled = providers.gradleProperty("alef.skipHostJni").orNull != "true"
        val rows = (consumers + unrelated).map {{ name ->
            val task = tasks.named(name).get()
            val actual = task.taskDependencies.getDependencies(task).map {{ it.name }}.toSet()
            Triple(name, actual, if (enabled && name in consumers) setOf("copyHostJni") else emptySet<String>())
        }}
        println("ALEF_RESOURCE_EDGE_ROWS=" + rows.size)
        rows.forEach {{ (name, actual, expected) -> println("EDGE:$name:$actual:$expected") }}
        check(rows.size == 6)
        val mismatches = rows.filter {{ it.second != it.third }}
        check(mismatches.isEmpty()) {{ "GENERATED_JNI_DEPENDENCY_MISMATCH: $mismatches" }}
        println("ALEF_RESOURCE_EDGES_QUALIFIED=6")
    }}
}}
"#,
        blocks[0]
    );
    std::fs::write(root.path().join("build.gradle.kts"), fixture).unwrap();
    std::fs::write(
        root.path().join("settings.gradle.kts"),
        "rootProject.name = \"generated-edge-probe\"\n",
    )
    .unwrap();
    std::fs::write(root.path().join("gradle.properties"),
        "org.gradle.daemon=false\norg.gradle.workers.max=1\norg.gradle.jvmargs=-Xmx512m\norg.gradle.java.installations.auto-download=false\n").unwrap();
    let gradle = which::which("gradle").expect("managed Gradle is required; run the explicit native gate with mise");
    for skip in [true, false] {
        let mut command = Command::new(&gradle);
        command
            .current_dir(root.path())
            .args([
                "--offline",
                "--no-daemon",
                "--max-workers=1",
                "--console=plain",
                "verifyGeneratedResourceEdges",
            ])
            .env("GRADLE_USER_HOME", root.path().join("gradle-home"));
        if let Some(java) = std::env::var_os("JAVA_HOME") {
            command.env("JAVA_HOME", java);
        }
        if skip {
            command.arg("-Palef.skipHostJni=true");
        }
        let output = run_command_streams(&mut command, 300).expect("bounded actual Gradle invocation");
        eprintln!("skipHostJni={skip}\n{}\n{}", output.stdout, output.stderr);
        if !output.success {
            let retained = root.keep();
            panic!(
                "actual generated dependency graph failed; retained {}",
                retained.display()
            );
        }
        assert!(
            output.stdout.contains("ALEF_RESOURCE_EDGE_ROWS=6"),
            "all six actual tasks must be inspected"
        );
        assert!(output.stdout.contains("ALEF_RESOURCE_EDGES_QUALIFIED=6"));
    }
}
