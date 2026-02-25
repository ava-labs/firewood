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

1. AWS CLI is installed ([instructions](https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html))
2. AWS credentials are configured in your shell environment. If not, follow:
   - Sign in via Okta (you need to go through Okta and click on AWS to open the accounts page).
   - You'll see a list of your AWS accounts — click on the account you want
   - Next to the role name, click "Access keys" (with a key icon)
   - A modal will pop up showing you credentials.
   - Run `aws configure sso` and fill in the credentials from previous step. You can also follow the instructions [here](https://docs.aws.amazon.com/cli/latest/userguide/sso-configure-profile-token.html#sso-configure-profile-token-auto-sso).
   - You can rename your profile to "default" in `~/.aws/config`, or add `export AWS_PROFILE=<profile-name>` to your shell configuration.
3. The target region has the security group you plan to use. The default security group is for `us-west-2` and provides ssh access. You can create a similar profile using AWS Console or from CLI using:

    ```sh
    # find default vpc
    DEFAULT_VPC=$(aws ec2 describe-vpcs \
        --region <your-region> \
        --filters "Name=isDefault,Values=true" \
        --query "Vpcs[0].VpcId" \
        --output text)
    # create new security group
    SG_ID=$(aws ec2 create-security-group \
        --region <your-region> \
        --group-name fwdctl-launch-sg \
        --description "Security group for SSH access of fwdctl/launch instances" \
        --vpc-id $DEFAULT_VPC \
        --query GroupId \
        --output text)
    # add ssh inbound rule (from anywhere)
    aws ec2 authorize-security-group-ingress \
        --region <your-region> \
        --group-id $SG_ID \
        --protocol tcp \
        --port 22 \
        --cidr 0.0.0.0/0
    ```

4. Your IAM identity can call EC2, SSM, and STS APIs used by launch flows. You can check with following commands, they should succeed:
    ```sh
    # should show user id
    aws sts get-caller-identity
    # should show instances, or empty
    aws ssm describe-instance-information --region <region>
    # ec2, should show instances or empty
    aws ec2 describe-instances --region <region>
    # ec2 dry-run. should error DryRunOperation: Request would have succeeded, but DryRun flag is set.
    aws ec2 run-instances \
        --image-id resolve:ssm:/aws/service/ami-amazon-linux-latest/al2023-ami-kernel-default-x86_64 \
        --instance-type t2.micro \
        --region us-west-2 \
        --dry-run
    ```

5. The IAM roles and profiles are global, the default one should work. If you decide to create a new one, you should first create a role and then an instance profile that the role should be added to it. You'll need to add s3 read access and ssm permissions to the role for the instance to be able to download blocks from s3, and be monitored by launch utility using SSM. Steps to create a new profile satisfying these criteria:

    ```bash
    # create the iam role
    aws iam create-role \
        --role-name <new-role-name> \
        --assume-role-policy-document '{
            "Version": "2012-10-17",
            "Statement": [
            {
                "Effect": "Allow",
                "Principal": {
                "Service": "ec2.amazonaws.com"
                },
                "Action": "sts:AssumeRole"
            }
            ]
        }'
    # attach s3 read policy
    aws iam attach-role-policy \
        --role-name <role> \
        --policy-arn arn:aws:iam::aws:policy/AmazonS3ReadOnlyAccess
    # attach ssm policy
    aws iam attach-role-policy \
        --role-name <role> \
        --policy-arn arn:aws:iam::aws:policy/AmazonSSMManagedInstanceCore
    # create instance profile
    aws iam create-instance-profile \
        --instance-profile-name <instance-profile-name>
    # add role to profile
    aws iam add-role-to-instance-profile \
        --instance-profile-name <profile> \
        --role-name <role>
    ```


## Quick start

1. Preview the launch plan without creating resources:

    ```sh
    ./target/release/fwdctl launch deploy --dry-run
    ```
   **Note:** You'll need to explicitly specify the AWS profile if you didn't set as default or export it as instructed previously:
    
   ```sh
    AWS_PROFILE=<profile> ./target/release/fwdctl launch deploy --dry-run
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
