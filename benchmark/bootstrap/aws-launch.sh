#!/usr/bin/env bash
set +e

# Default values
INSTANCE_TYPE=i4g.large
FIREWOOD_BRANCH=""
AVALANCHEGO_BRANCH=""
LIBEVM_BRANCH=""
NBLOCKS="1m"
CONFIG="firewood"
PROFILE=""
REGION="us-west-2"
DRY_RUN=false
SPOT_INSTANCE=false
SHOW_INSTANCES=false
TERMINATE_MINE=false
TERMINATE_INSTANCES=()
COMMON_ARGS=()

# Valid instance types and their architectures
declare -A VALID_INSTANCES=(
    ["i4g.large"]="arm64"
    ["i4g.xlarge"]="arm64"
    ["i4i.large"]="amd64"
    ["i4i.xlarge"]="amd64"
    ["m6id.xlarge"]="amd64"
    ["c6gd.2xlarge"]="arm64"
    ["x2gd.xlarge"]="arm64"
    ["m5ad.2xlarge"]="amd64"
    ["r6gd.2xlarge"]="arm64"
    ["r6id.2xlarge"]="amd64"
    ["x2gd.2xlarge"]="arm64"
    ["z1d.2xlarge"]="amd64"
    ["i8ge.12xlarge"]="arm64"
)

# Maximum spot prices for each instance type (from the pricing table)
declare -A MAX_SPOT_PRICES=(
    ["i4g.large"]="0.1544"
    ["i4g.xlarge"]="0.3088"
    ["i4i.large"]="0.1720"
    ["i4i.xlarge"]="0.3440"
    ["m6id.xlarge"]="0.2373"
    ["c6gd.2xlarge"]="0.3072"
    ["x2gd.xlarge"]="0.3340"
    ["m5ad.2xlarge"]="0.4120"
    ["r6gd.2xlarge"]="0.4608"
    ["r6id.2xlarge"]="0.6048"
    ["x2gd.2xlarge"]="0.6680"
    ["z1d.2xlarge"]="0.7440"
    ["i8ge.12xlarge"]="5.6952"
)

# Valid nblocks values
VALID_NBLOCKS=("1m" "10m" "50m")

# Function to show usage
show_usage() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --instance-type TYPE        EC2 instance type (default: i4g.large)"
    echo "  --firewood-branch BRANCH    Firewood git branch to checkout"
    echo "  --avalanchego-branch BRANCH AvalancheGo git branch to checkout"
    echo "  --libevm-branch BRANCH      LibEVM git branch to checkout"
    echo "  --nblocks BLOCKS            Number of blocks to download (default: 1m)"
    echo "  --config CONFIG             The VM reexecution config to use (default: firewood)"
    echo "  --profile PROFILE           AWS CLI profile to use (default: AWS default)"
    echo "  --region REGION             AWS region (default: us-west-2)"
    echo "  --spot                      Use spot instance pricing (default depends on instance type)"
    echo "  --dry-run                   Show the aws command that would be run without executing it"
    echo "  --show                      Show currently running instances in the region"
    echo "  --terminate-mine            Terminate all running instances created by current user"
    echo "  --terminate ID [ID...]      Terminate specific instance(s) by instance ID"
    echo "  --help                      Show this help message"
    echo ""
    echo "Valid instance types:"
    echo "  # name         Type  disk vcpu memory   $/hr    notes"
    echo "  i4g.large      arm64 468  2    16 GiB   \$0.1544 Graviton2-powered"
    echo "  i4i.large      amd64 468  2    16 GiB   \$0.1720 Intel Xeon Scalable"
    echo "  m6id.xlarge    amd64 237  4    16 GiB   \$0.2373 Intel Xeon Scalable"
    echo "  c6gd.2xlarge   arm64 474  8    16 GiB   \$0.3072 Graviton2 compute-optimized"
    echo "  i4g.xlarge     arm64 937  4    32 GiB   \$0.3088 Graviton2-powered"
    echo "  i4i.xlarge     amd64 937  4    32 GiB   \$0.3440 Intel Xeon Scalable"
    echo "  x2gd.xlarge    arm64 237  4    64 GiB   \$0.3340 Graviton2 memory-optimized"
    echo "  m5ad.2xlarge   amd64 300  8    32 GiB   \$0.4120 AMD EPYC processors"
    echo "  r6gd.2xlarge   arm64 474  8    64 GiB   \$0.4608 Graviton2 memory-optimized"
    echo "  r6id.2xlarge   amd64 474  8    64 GiB   \$0.6048 Intel Xeon Scalable"
    echo "  x2gd.2xlarge   arm64 475  8    128 GiB  \$0.6680 Graviton2 memory-optimized"
    echo "  z1d.2xlarge    amd64 300  8    64 GiB   \$0.7440 High-frequency Intel Xeon CPUs"
    echo "  i8ge.12xlarge  arm64 11250 48  384 GiB  \$5.5952 Careful, very expensive"
    echo ""
    echo "Valid nblocks values: ${VALID_NBLOCKS[*]}"
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --instance-type)
            INSTANCE_TYPE="$2"
            if [[ ! ${VALID_INSTANCES[$INSTANCE_TYPE]+_} ]]; then
                echo "Error: Invalid instance type '$INSTANCE_TYPE'"
                echo "Valid types: ${!VALID_INSTANCES[*]}"
                exit 1
            fi
            shift 2
            ;;
        --firewood-branch)
            FIREWOOD_BRANCH="$2"
            shift 2
            ;;
        --avalanchego-branch)
            AVALANCHEGO_BRANCH="$2"
            shift 2
            ;;
        --libevm-branch)
            LIBEVM_BRANCH="$2"
            shift 2
            ;;
        --nblocks)
            NBLOCKS="$2"
            # Validate nblocks value
            if [[ ! " ${VALID_NBLOCKS[*]} " =~ [[:space:]]${NBLOCKS}[[:space:]] ]]; then
                echo "Error: Invalid nblocks value '$NBLOCKS'"
                echo "Valid values: ${VALID_NBLOCKS[*]}"
                exit 1
            fi
            shift 2
            ;;
        --config)
            CONFIG="$2"
            shift 2
            ;;
        --profile)
            PROFILE="$2"
            shift 2
            ;;
        --region)
            REGION="$2"
            shift 2
            ;;
        --spot)
            SPOT_INSTANCE=true
            shift
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        --show)
            SHOW_INSTANCES=true
            shift
            ;;
        --terminate-mine)
            TERMINATE_MINE=true
            shift
            ;;
        --terminate)
            shift
            # Collect all following arguments as instance IDs until we hit another flag or end
            while [[ $# -gt 0 ]] && [[ ! $1 =~ ^-- ]]; do
                TERMINATE_INSTANCES+=("$1")
                shift
            done
            if [[ ${#TERMINATE_INSTANCES[@]} -eq 0 ]]; then
                echo "Error: --terminate requires at least one instance ID"
                exit 1
            fi
            ;;
        --help)
            show_usage
            exit 0
            ;;
        *)
            echo "Error: Unknown option $1"
            show_usage
            exit 1
            ;;
    esac
done

if [ -n "$PROFILE" ]; then
    COMMON_ARGS+=("--profile" "$PROFILE")
fi
COMMON_ARGS+=("--region" "$REGION")

# Handle --terminate option
if [ ${#TERMINATE_INSTANCES[@]} -gt 0 ]; then
    # Check if any incompatible options are present
    if [ -n "$FIREWOOD_BRANCH" ] || [ -n "$AVALANCHEGO_BRANCH" ] || \
       [ -n "$LIBEVM_BRANCH" ] || [ "$INSTANCE_TYPE" != "i4g.large" ] || [ "$NBLOCKS" != "1m" ] || \
       [ "$CONFIG" != "firewood" ] || [ "$DRY_RUN" = true ] || [ "$SPOT_INSTANCE" = true ] || \
       [ "$SHOW_INSTANCES" = true ] || [ "$TERMINATE_MINE" = true ]; then
        echo "Error: --terminate cannot be used with other options except --profile and --region"
        exit 1
    fi
    
    echo "Terminating instances: ${TERMINATE_INSTANCES[*]}"
    aws ec2 terminate-instances \
      "${COMMON_ARGS[*]}" \
      --instance-ids "${TERMINATE_INSTANCES[@]}"
    exit 0
fi

# Handle --terminate-mine option
if [ "$TERMINATE_MINE" = true ]; then
    # Check if any incompatible options are present
    if [ -n "$FIREWOOD_BRANCH" ] || [ -n "$AVALANCHEGO_BRANCH" ] || \
       [ -n "$LIBEVM_BRANCH" ] || [ "$INSTANCE_TYPE" != "i4g.large" ] || [ "$NBLOCKS" != "1m" ] || \
       [ "$CONFIG" != "firewood" ] || [ "$DRY_RUN" = true ] || [ "$SPOT_INSTANCE" = true ] || \
       [ "$SHOW_INSTANCES" = true ]; then
        echo "Error: --terminate-mine cannot be used with other options except --profile and --region"
        exit 1
    fi
    
    # Get instances created by current user (Name tag starts with username)
    INSTANCE_IDS=$(aws ec2 describe-instances \
      ${COMMON_ARGS[*]} \
      --filters "Name=tag:Name,Values=$USER-*" "Name=instance-state-name,Values=running" \
      --query "Reservations[*].Instances[*].InstanceId" \
      --output text)
    
    if [ -z "$INSTANCE_IDS" ]; then
        echo "No running instances found for user: $USER"
        exit 0
    fi
    
    echo "Terminating instances for user $USER: $INSTANCE_IDS"
    aws ec2 terminate-instances \
      ${COMMON_ARGS[*]} \
      --instance-ids $INSTANCE_IDS
    exit 0
fi

# Handle --show option
if [ "$SHOW_INSTANCES" = true ]; then
    # Check if any incompatible options are present
    if [ -n "$FIREWOOD_BRANCH" ] || [ -n "$AVALANCHEGO_BRANCH" ] || \
       [ -n "$LIBEVM_BRANCH" ] || [ "$INSTANCE_TYPE" != "i4g.large" ] || [ "$NBLOCKS" != "1m" ] || \
       [ "$CONFIG" != "firewood" ] || [ "$DRY_RUN" = true ] || [ "$SPOT_INSTANCE" = true ]; then
        echo "Error: --show cannot be used with other options except --region"
        exit 1
    fi
    
    # Execute the AWS command to show instances
    aws ec2 describe-instances \
      ${COMMON_ARGS[*]} \
      --filters "Name=key-name,Values=rkuris" "Name=instance-state-name,Values=running" \
      --query "Reservations[*].Instances[?PublicIpAddress!=null].[InstanceId, LaunchTime, PublicIpAddress, InstanceType, Tags[?Key=='Name']|[0].Value]" \
      --output json | jq -r '.[][] | @sh'
    exit 0
fi

# Set architecture type based on instance type
TYPE=${VALID_INSTANCES[$INSTANCE_TYPE]}

echo "Configuration:"
echo "  Instance Type: $INSTANCE_TYPE ($TYPE)"
echo "  Firewood Branch: ${FIREWOOD_BRANCH:-default}"
echo "  AvalancheGo Branch: ${AVALANCHEGO_BRANCH:-default}"
echo "  LibEVM Branch: ${LIBEVM_BRANCH:-default}"
echo "  Number of Blocks: $NBLOCKS"
echo "  Config: $CONFIG"
echo "  AWS Profile: $PROFILE"
echo "  Region: $REGION"
if [ "$SPOT_INSTANCE" = true ]; then
    echo "  Spot Instance: Yes (max price: \$${MAX_SPOT_PRICES[$INSTANCE_TYPE]})"
else
    echo "  Spot Instance: No"
fi
if [ "$DRY_RUN" = true ]; then
    echo "  Mode: DRY RUN (will not launch instance)"
fi
echo ""

if [ "$DRY_RUN" = true ]; then
    # For dry run, use placeholder values
    AMI_ID="ami-placeholder"
    USERDATA="base64-encoded-userdata-placeholder"
else
    # find the latest ubuntu-noble base image ID (only works for intel processors)
    AMI_ID=$(aws ec2 describe-images \
        ${COMMON_ARGS[*]} \
        --owners 099720109477 \
        --filters "Name=name,Values=ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-$TYPE-server-*" \
                  "Name=state,Values=available" \
        --query "Images | sort_by(@, &CreationDate)[-1].ImageId" \
        --output text)
    export AMI_ID
fi

if [ "$DRY_RUN" = false ]; then
    # Prepare branch arguments for cloud-init
FIREWOOD_BRANCH_ARG=""
AVALANCHEGO_BRANCH_ARG=""

if [ -n "$FIREWOOD_BRANCH" ]; then
    FIREWOOD_BRANCH_ARG="--branch $FIREWOOD_BRANCH"
fi
if [ -n "$AVALANCHEGO_BRANCH" ]; then
    AVALANCHEGO_BRANCH_ARG="--branch $AVALANCHEGO_BRANCH"
fi

# For libevm, default to main branch if no branch specified
LIBEVM_BRANCH_CHECKOUT="${LIBEVM_BRANCH:-main}"

# set up this script to run at startup, installing a few packages, creating user accounts,
# and downloading the blocks for the C-chain
USERDATA_TEMPLATE=$(cat <<'END_HEREDOC'
#cloud-config
package_update: true
package_upgrade: true
packages:
  - git
  - build-essential
  - curl
  - protobuf-compiler
  - make
  - apt-transport-https
  - net-tools
  - unzip
users:
  - default
  - name: rkuris
    lock_passwd: true
    groups: users, adm, sudo
    shell: /usr/bin/bash
    sudo: "ALL=(ALL) NOPASSWD:ALL"
    ssh_authorized_keys:
      - ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIL2RVmfpoKYi0tJd2DhQEp8tB3m2PSuaYxIfnLwqt03u cardno:23_537_110 ron
  - name: austin
    lock_passwd: true
    groups: users, adm, sudo
    shell: /usr/bin/bash
    sudo: "ALL=(ALL) NOPASSWD:ALL"
    ssh_authorized_keys:
      - ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICoGgX8nCin3FPc1V3YYN1M9g039wMbzZSAXZJCqzBt3 cardno:31_786_961 austin
  - name: aaron
    lock_passwd: true
    groups: users, adm, sudo
    shell: /usr/bin/bash
    sudo: "ALL=(ALL) NOPASSWD:ALL"
    ssh_authorized_keys:
      - ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMj2j6ySwsFx7Y6FW2UXlkjCZfFDQKHWh0GTBjkK9ruV cardno:19_236_959 aaron
  - name: brandon
    lock_passwd: true
    groups: users, adm, sudo
    shell: /usr/bin/bash
    sudo: "ALL=(ALL) NOPASSWD:ALL"
    ssh_authorized_keys:
      - ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHWuCq/y50S0yFPJQEAifeeN4n6EL3IlUuYbAdk2w2kL cardno:33_731_913 brandon
  - name: amin
    lock_passwd: true
    groups: users, adm, sudo
    shell: /usr/bin/bash
    sudo: "ALL=(ALL) NOPASSWD:ALL"
    ssh_authorized_keys:
      - ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIE8iR1X8/ELrzjczZvCkrTGCEoN6/dtlP01QFGuUpYxV cardno:33_317_839 amin
  - name: bernard
    lock_passwd: true
    groups: users, adm, sudo
    shell: /usr/bin/bash
    sudo: "ALL=(ALL) NOPASSWD:ALL"
    ssh_authorized_keys:
      - ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIE/1C8JVL0g6qqMw1p0TwJMqJqERxYTX+7PnP+gXP4km cardno:19_155_748 bernard
  - name: rodrigo
    lock_passwd: true
    groups: users, adm, sudo
    shell: /usr/bin/bash
    sudo: "ALL=(ALL) NOPASSWD:ALL"
    ssh_authorized_keys:
      - ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDT0/IE2kLNpvaELug1zppQGY03z3fe2zOTjyS655Sgq cardno:28_650_437 rodrigo

swap:
  filename: /swapfile
  size: 16G
  maxsize: 16G

# anyone can use the -D option
write_files:
- content: |
   Defaults runcwd=*
  path: "/etc/sudoers.d/91-cloud-init-enable-D-option"
  permissions: '0440'
- content: |
    export PATH="$PATH:/usr/local/go/bin"
  permissions: "0644"
  path: "/etc/profile.d/go_path.sh"
- content: |
    export RUSTUP_HOME=/usr/local/rust
    export PATH="$PATH:/usr/local/rust/bin"
  permissions: "0644"
  path: "/etc/profile.d/rust_path.sh"

runcmd:
  # install rust
  - echo 'PATH=/usr/local/go/bin:$HOME/.cargo/bin:$PATH' >> ~ubuntu/.profile
  - >
    curl https://sh.rustup.rs -sSf
    | RUSTUP_HOME=/usr/local/rust CARGO_HOME=/usr/local/rust
    sh -s -- -y --no-modify-path
  - sudo -u ubuntu --login rustup default stable
  # install firewood
  - git clone --depth 1 __FIREWOOD_BRANCH_ARG__ https://github.com/ava-labs/firewood.git /tmp/firewood
  - bash /tmp/firewood/benchmark/setup-scripts/build-environment.sh
  - bash -c 'mkdir ~ubuntu/firewood; mv /tmp/firewood/{.[!.],}* ~ubuntu/firewood/'
  # fix up the directories so that anyone is group 'users' has r/w access
  - chown -R ubuntu:users /mnt/nvme/ubuntu
  - chmod -R g=u /mnt/nvme/ubuntu
  - find /mnt/nvme/ubuntu -type d -print0 | xargs -0 chmod g+s
  # helpful symbolic links from home directories
  - sudo -u ubuntu ln -s /mnt/nvme/ubuntu/data /home/ubuntu/data
  - sudo -u ubuntu ln -s /mnt/nvme/ubuntu/avalanchego /home/ubuntu/avalanchego
  # install go and grafana
  - bash /mnt/nvme/ubuntu/firewood/benchmark/setup-scripts/install-golang.sh
  - bash /mnt/nvme/ubuntu/firewood/benchmark/setup-scripts/install-grafana.sh
  # install task, avalanchego, libevm
  - snap install task --classic
  - >
    sudo -u ubuntu -D /mnt/nvme/ubuntu
    git clone --depth 100 __AVALANCHEGO_BRANCH_ARG__ https://github.com/ava-labs/avalanchego.git
  - >
    sudo -u ubuntu -D /mnt/nvme/ubuntu
    git clone https://github.com/ava-labs/libevm.git
  - >
    sudo -u ubuntu -D /mnt/nvme/ubuntu/libevm
    git checkout __LIBEVM_BRANCH__
  # force coreth to use the checked-out version of firewood and libevm
  - >
    sudo -u ubuntu -D /mnt/nvme/ubuntu/avalanchego/graft/coreth
    /usr/local/go/bin/go mod edit -replace
    github.com/ava-labs/firewood-go-ethhash/ffi=../../../firewood/ffi
  - >
    sudo -u ubuntu -D /mnt/nvme/ubuntu/avalanchego/graft/coreth
    /usr/local/go/bin/go mod edit -replace
    github.com/ava-labs/libevm=../../../libevm
  # force avalanchego to use the checked-out versions of libevm and firewood
  - >
    sudo -u ubuntu -D /mnt/nvme/ubuntu/avalanchego
    /usr/local/go/bin/go mod edit -replace
    github.com/ava-labs/firewood-go-ethhash/ffi=../firewood/ffi
  - >
    sudo -u ubuntu -D /mnt/nvme/ubuntu/avalanchego
    /usr/local/go/bin/go mod edit -replace
    github.com/ava-labs/libevm=../libevm
  # build firewood in maxperf mode
  - >
    sudo -u ubuntu -D /mnt/nvme/ubuntu/firewood/ffi --login time cargo build
    --profile maxperf
    --features ethhash,logger
    > /mnt/nvme/ubuntu/firewood/build.log 2>&1
  # run go mod tidy for coreth and avalanchego
  - sudo -u ubuntu --login -D /mnt/nvme/ubuntu/avalanchego/graft/coreth go mod tidy
  - sudo -u ubuntu --login -D /mnt/nvme/ubuntu/avalanchego go mod tidy
  # build avalanchego
  - >
    sudo -u ubuntu --login -D /mnt/nvme/ubuntu/avalanchego time scripts/build.sh
    > /mnt/nvme/ubuntu/avalanchego/build.log 2>&1 &
  # install s5cmd
  - curl -L -o /tmp/s5cmd.deb $(curl -s https://api.github.com/repos/peak/s5cmd/releases/latest | grep "browser_download_url" | grep "linux_$(dpkg --print-architecture).deb" | cut -d '"' -f 4) && dpkg -i /tmp/s5cmd.deb
  # download and extract mainnet blocks
  - echo 'downloading mainnet blocks'
  - sudo -u ubuntu mkdir -p /mnt/nvme/ubuntu/exec-data/blocks
  - s5cmd cp s3://avalanchego-bootstrap-testing/cchain-mainnet-blocks-__NBLOCKS__-ldb/\* /mnt/nvme/ubuntu/exec-data/blocks/ >/dev/null
  - chown -R ubuntu /mnt/nvme/ubuntu/exec-data
  - chmod -R g=u /mnt/nvme/ubuntu/exec-data
  # execute bootstrapping
  - >
    sudo -u ubuntu -D /mnt/nvme/ubuntu/avalanchego --login
    time task reexecute-cchain-range CURRENT_STATE_DIR=/mnt/nvme/ubuntu/exec-data/current-state BLOCK_DIR=/mnt/nvme/ubuntu/exec-data/blocks START_BLOCK=1 END_BLOCK=__END_BLOCK__ CONFIG=__CONFIG__ METRICS_ENABLED=false
    > /var/log/bootstrap.log 2>&1
END_HEREDOC
)

# Convert nblocks to actual end block number
case $NBLOCKS in
    "1m")   END_BLOCK="1000000" ;;
    "10m")  END_BLOCK="10000000" ;;
    "50m")  END_BLOCK="50000000" ;;
    *)      END_BLOCK="1000000" ;;  # Default fallback
esac

# Substitute branch arguments and block values in the userdata template
USERDATA=$(echo "$USERDATA_TEMPLATE" | \
  sed "s|__FIREWOOD_BRANCH_ARG__|$FIREWOOD_BRANCH_ARG|g" | \
  sed "s|__AVALANCHEGO_BRANCH_ARG__|$AVALANCHEGO_BRANCH_ARG|g" | \
  sed "s|__LIBEVM_BRANCH__|$LIBEVM_BRANCH_CHECKOUT|g" | \
  sed "s|__NBLOCKS__|$NBLOCKS|g" | \
  sed "s|__END_BLOCK__|$END_BLOCK|g" | \
  sed "s|__CONFIG__|$CONFIG|g" | \
  base64)
export USERDATA

fi  # End of DRY_RUN=false conditional


# Generate a compact 8-hex-digit suffix without spaces/newlines
SUFFIX=$(hexdump -vn4 -e '1/4 "%08X"' /dev/urandom)

# Build instance name with branch info
INSTANCE_NAME="$USER-fw-$SUFFIX"
if [ -n "$FIREWOOD_BRANCH" ]; then
    INSTANCE_NAME="$INSTANCE_NAME-fw-$FIREWOOD_BRANCH"
fi
if [ -n "$AVALANCHEGO_BRANCH" ]; then
    INSTANCE_NAME="$INSTANCE_NAME-ag-$AVALANCHEGO_BRANCH"
fi
if [ -n "$LIBEVM_BRANCH" ]; then
    INSTANCE_NAME="$INSTANCE_NAME-le-$LIBEVM_BRANCH"
fi

# Build spot instance market options if requested
SPOT_OPTIONS=""
if [ "$SPOT_INSTANCE" = true ]; then
    MAX_PRICE=${MAX_SPOT_PRICES[$INSTANCE_TYPE]}
    SPOT_OPTIONS="--instance-market-options '{\"MarketType\":\"spot\", \"SpotOptions\": {\"MaxPrice\":\"$MAX_PRICE\"}}'"
fi

if [ "$DRY_RUN" = true ]; then
    echo "DRY RUN - Would execute the following command:"
    echo ""
    echo "aws ec2 run-instances ${COMMON_ARGS[*]} \\"
    echo "  --image-id \"$AMI_ID\" \\"
    echo "  --count 1 \\"
    echo "  --instance-type $INSTANCE_TYPE \\"
    echo "  --key-name rkuris \\"
    echo "  --security-groups rkuris-starlink-only \\"
    echo "  --iam-instance-profile \"Name=s3-readonly\" \\"
    if [ "$SPOT_INSTANCE" = true ]; then
        echo "  $SPOT_OPTIONS \\"
    fi
    echo "  --user-data \"$USERDATA\" \\"
    echo "  --tag-specifications \"ResourceType=instance,Tags=[{Key=Name,Value=$INSTANCE_NAME}]\" \\"
    echo "  --block-device-mappings \"DeviceName=/dev/sda1,Ebs={VolumeSize=50,VolumeType=gp3}\" \\"
    echo "  --query 'Instances[0].InstanceId' \\"
    echo "  --output text"
    echo ""
    echo "Instance would be named: $INSTANCE_NAME"
else
    set -e
    # Build the AWS command with conditional spot options
    AWS_CMD="aws ec2 run-instances \
        ${COMMON_ARGS[*]} \
      --region \"$REGION\" \
      --image-id \"$AMI_ID\" \
      --count 1 \
      --instance-type \"$INSTANCE_TYPE\" \
      --key-name rkuris \
      --security-groups rkuris-starlink-only \
      --iam-instance-profile \"Name=s3-readonly\""

    if [ "$SPOT_INSTANCE" = true ]; then
        AWS_CMD="$AWS_CMD $SPOT_OPTIONS"
    fi

    AWS_CMD="$AWS_CMD \
      --user-data \"$USERDATA\" \
      --tag-specifications \"ResourceType=instance,Tags=[{Key=Name,Value=$INSTANCE_NAME}]\" \
      --block-device-mappings \"DeviceName=/dev/sda1,Ebs={VolumeSize=50,VolumeType=gp3}\" \
      --query 'Instances[0].InstanceId' \
      --output text"

    INSTANCE_ID=$(eval "$AWS_CMD")
    echo "instance id $INSTANCE_ID started"
    echo -e "\033[33mREMINDER: Please manually shut down your EC2 instance after running the benchmark to avoid unexpected costs!\033[0m"
fi

if [ "$DRY_RUN" = false ]; then
    # IP=$(aws ec2 describe-instances --profile "$PROFILE" --instance-ids "$INSTANCE_ID" --query 'Reservations[].Instances[].PublicIpAddress' --output text)
    set +e

    #IP=$(echo "$JSON" | jq -r '.PublicIpAddress')
    #echo $IP
    #while nc -zv $IP 22; do
        #sleep 1
    #done
fi
