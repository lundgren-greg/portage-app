let meta = #{
    name: "next-pr",
    description: "Implement the next Portage design PR only: scope, implement, cargo test, review",
    when_to_use: "Start or continue the next numbered PR in docs/design.md",
    phases: [
        #{ title: "Scope", detail: "read PROJECT.md and the next PR block" },
        #{ title: "Implement", detail: "TDD that PR only on a feat/prN branch" },
        #{ title: "Verify", detail: "cargo test, clippy, fmt" },
        #{ title: "Review", detail: "independent read of the diff" },
    ],
};

phase("Scope");
let scope = agent(
    "You are scoping the next Portage PR. Read C:\\Repos\\portage-app\\PROJECT.md and the PR Plan at the bottom of C:\\Repos\\portage-app\\docs\\design.md using read_file. Identify the next unmerged numbered PR. Read that PR's Description and Files/components. Also run git -C C:\\Repos\\portage-app status -sb and git -C C:\\Repos\\portage-app log -1 --oneline. Reply with: PR number, title, files, and one-sentence done criterion. Do not implement.",
    #{ label: "scope", capability_mode: "read-only" }
);
if scope == () || !scope.success {
    pause("verification", "Scope agent failed — resume after PROJECT.md and the design PR Plan are readable.");
}

phase("Implement");
let impl_prompt = "";
impl_prompt += "Implement ONLY the next Portage PR identified here:\n";
impl_prompt += json_encode(scope.output);
impl_prompt += "\nFollow C:\\Repos\\portage-app\\AGENTS.md and .agents/skills/next-pr/SKILL.md. ";
impl_prompt += "Repo is C:\\Repos\\portage-app. Use tdd: failing test first. ";
impl_prompt += "Do not implement later PRs. Do not add share-link APIs. Do not open placeholders. ";
impl_prompt += "The LLM never applies a plan. Update PROJECT.md Stopped at when the slice is done.";
let built = agent(impl_prompt, #{ label: "implement", capability_mode: "all" });
if built == () || !built.success {
    pause("no_progress", "Implementer failed. Resume after inspecting the working tree.");
}

phase("Verify");
let verified = agent(
    "In C:\\Repos\\portage-app run these commands and report pass/fail with output tails: cargo test --workspace ; cargo clippy --workspace -- -D warnings ; cargo fmt --all -- --check. Do not claim green unless the commands succeeded. Do not change product behavior unless a test or clippy failure requires a one-line fix.",
    #{ label: "verify", capability_mode: "execute" }
);
if verified == () || !verified.success {
    pause("verification", "Verify agent failed to run cargo.");
}

phase("Review");
let review = agent(
    "Review the uncommitted and branch diff in C:\\Repos\\portage-app against docs/design.md for this PR only. Use git diff and read_file. Look for missing tests, last-copy / placeholder / share-link violations, and scope creep. List concrete issues or say none after reading the code.",
    #{ label: "review", capability_mode: "read-only" }
);

complete(#{
    summary: "next-pr finished scope + implement + verify + review",
    scope: scope.output,
    implement: built.output,
    verify: verified.output,
    review: review.output,
});
