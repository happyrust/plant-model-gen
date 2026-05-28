# Remote Offline Deploy Goal

Build a Linux MVP for offline remote deployment of existing managed project sites. The local admin UI should guide users through deploying a selected site to Ubuntu 22 or CentOS 7.9 targets over SSH, support root/sudo and degraded ordinary-user deployments, and expose enough remote site-agent status for monitoring after installation.

The shared understanding is captured in `facts.md`. The approved execution plan is captured in `plan.md`.

## Done Condition

- `/offline-deploy` is the primary admin entry for remote offline deployment.
- Existing managed sites can be deployed to Linux targets through preflight, upload, remote config, service/process start, and validation.
- Root or sudo deployments install system services; ordinary non-sudo deployments use generated user-mode scripts and are clearly marked degraded.
- Remote deployment status is associated with the submitted task or deploy id and distinguishes full success from degraded success.
- A deployed remote `web_server` exposes site-agent status with site identity, deploy/version metadata, runtime mode, health, and token-based identity fields.
- Real remote smoke verification is recorded for Ubuntu 22 before MVP completion; CentOS 7.9 is only marked complete after its own real smoke passes.
- Verification uses a running `web_server` plus HTTP/POST checks, not cargo tests.

Launch with:

```text
/goal goals/remote-offline-deploy/goal.md
```
