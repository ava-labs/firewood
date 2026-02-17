# fwdctl launch

`fwdctl launch` runs benchmark workflows on AWS EC2 and provides follow-up commands
for monitoring, listing, and terminating managed instances.

## Build and help

The launch subcommand is behind the `launch` feature flag.

```sh
cargo build --release --bin fwdctl --features launch
./target/release/fwdctl launch -h
./target/release/fwdctl launch deploy -h
```

## AWS prerequisites

Before using launch commands, make sure:

1. AWS credentials are configured in your shell environment.
2. The target region has the security group and optional key pair you plan to use.
3. Your IAM identity can call EC2, SSM, and STS APIs used by launch flows.
4. The IAM instance profile used at launch time is available in the target region.

## Quick start

1. Preview the launch plan without creating resources:

    ```sh
    ./target/release/fwdctl launch deploy --dry-run
    ```

2. Deploy and follow cloud-init stage progress:

    ```sh
    ./target/release/fwdctl launch deploy \
      --scenario reexecute \
      --nblocks 1m \
      --config firewood \
      --follow follow-with-progress
    ```

3. List instances launched through `fwdctl`:

    ```sh
    ./target/release/fwdctl launch list --running --mine
    ```

4. Re-attach to monitoring for one instance:

    ```sh
    ./target/release/fwdctl launch monitor i-0123456789abcdef0 --observe
    ```

5. Terminate one or more managed instances:

    ```sh
    ./target/release/fwdctl launch kill i-0123456789abcdef0 -y
    ./target/release/fwdctl launch kill --mine -y
    ```

## Deploy defaults

`launch deploy` has defaults to reduce required flags:

| Option | Default |
| --- | --- |
| `--instance-type` | `i4g.large` |
| `--nblocks` | `1m` |
| `--scenario` | `reexecute` |
| `--config` | `firewood` |
| `--metrics-server` | `true` |
| `--region` | `us-west-2` |
| `--sg` | `sg-0ac5ceb1761087d04` |
| `--iam-instance-profile` | `s3-readonly-with-ssm` |
| `--name-prefix` | `fw` |

Useful optional controls:

- `--dry-run` (`plan` or `plan-with-cloud-init`) to preview without creating resources.
- `--follow` (`follow` or `follow-with-progress`) to stream stage logs after deploy.
- `--firewood-branch`, `--avalanchego-branch`, `--libevm-branch` to test branch combinations.
- `--tag` to mark instances with a custom value (`CustomTag`).
- `--variable KEY=VALUE` to override `variables.<key>` in stage templates (repeatable; applied last; last value wins).

```sh
# Override via explicit key/value form (repeatable).
./target/release/fwdctl launch deploy \
  --variable nvme_base=/data/nvme \
  --variable s3_bucket=my-bootstrap-bucket
```

## Stage scenarios and config

By default, `fwdctl launch` uses the embedded stage config from:

- `benchmark/launch/launch-stages.yaml`

At runtime, you can override this without rebuilding by placing a file at:

- `~/.config/fwdctl/launch-stages.yaml`

Current embedded scenario names:

- `reexecute`
- `replay-log-gen`
- `snapshotter`

## Operational behavior

- Managed instances are tagged with `ManagedBy=fwdctl`, and only those instances are
  targeted by `launch list` and `launch kill`.
- `launch list` marks instances launched by your current AWS identity with `*`.
- `launch kill` requires explicit confirmation unless `-y`/`--yes` is provided.
