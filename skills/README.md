# Skills

Agent skills for usd-content-workspace. Back to the [usd-content-workspace README](../README.md).

- [usd-content-workspace-expert](usd-content-workspace-expert/SKILL.md) — the entry point. Chooses the services a workflow needs and routes to the component skills below. [AGENTS.md](../AGENTS.md) makes an agent in this repository act as this expert.
- [local-process-stack](local-process-stack/SKILL.md) — run or build Discovery, Storage, and Metadata as local processes, without Kubernetes.

Component skills are released with their components and stay beside the documentation they map:

- [Storage API contract map](../storage/api/skills/storage-api-contract-map/SKILL.md)
- [Storage Service documentation map](../storage/reference/storage-service/skills/storage-service/SKILL.md)
- [User Info Service documentation map](../user-identity/reference/microsoft-graph-userinfo-service/skills/userinfo-service-docs-map/SKILL.md)

The [storage build and configuration pointers](../storage/skills/README.md) are documentation, not skills.

`.agents/skills` and `.claude/skills` link to this directory so agents discover the skills automatically.
