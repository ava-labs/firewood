#!/usr/bin/env bash
set +e

# Default values
INSTANCE_TYPE=i4g.large
FIREWOOD_BRANCH=""
AVALANCHEGO_BRANCH=""
CORETH_BRANCH=""
LIBEVM_BRANCH=""
NBLOCKS="1m"
REGION="us-west-2"
DRY_RUN=false
SPOT_INSTANCE=false

# Instance information: architecture:spot_price:swap_size:disk:vcpu:memory:notes
declare -A INSTANCE_INFO=(
    ["i4g.large"]="arm64:0.1544:32G:468:2:16 GiB:Graviton2-powered"
    ["i4i.large"]="amd64:0.1720:32G:468:2:16 GiB:Intel Xeon Scalable"
    ["m6id.xlarge"]="amd64:0.2373:32G:237:4:16 GiB:Intel Xeon Scalable"
    ["c6gd.2xlarge"]="arm64:0.3072:32G:474:8:16 GiB:Graviton2 compute-optimized"
    ["i4g.xlarge"]="arm64:0.3088:16G:937:4:32 GiB:Graviton2-powered"
    ["x2gd.xlarge"]="arm64:0.3340:0:237:4:64 GiB:Graviton2 memory-optimized"
    ["i4i.xlarge"]="amd64:0.3440:16G:937:4:32 GiB:Intel Xeon Scalable"
    ["m5ad.2xlarge"]="arm64:0.4120:16G:300:8:32 GiB:AMD EPYC processors"
    ["r6gd.2xlarge"]="arm64:0.4608:0:474:8:64 GiB:Graviton2 memory-optimized"
    ["r6id.2xlarge"]="amd64:0.6048:0:474:8:64 GiB:Intel Xeon Scalable"
    ["x2gd.2xlarge"]="arm64:0.6680:0:475:8:128 GiB:Graviton2 memory-optimized"
    ["z1d.2xlarge"]="amd64:0.7440:0:300:8:64 GiB:High-frequency Intel Xeon CPUs"
    ["i8ge.12xlarge"]="arm64:5.6952:0:11250:48:384 GiB:Careful, very expensive"
)

# Valid nblocks values
VALID_NBLOCKS=("1m" "10m" "50m")

# Helper functions to extract instance information
get_instance_arch() {
    local instance_type=$1
    echo "${INSTANCE_INFO[$instance_type]}" | cut -d: -f1
}

get_instance_spot_price() {
    local instance_type=$1
    echo "${INSTANCE_INFO[$instance_type]}" | cut -d: -f2
}

get_instance_swap_size() {
    local instance_type=$1
    echo "${INSTANCE_INFO[$instance_type]}" | cut -d: -f3
}

get_instance_disk() {
    local instance_type=$1
    echo "${INSTANCE_INFO[$instance_type]}" | cut -d: -f4
}

get_instance_vcpu() {
    local instance_type=$1
    echo "${INSTANCE_INFO[$instance_type]}" | cut -d: -f5
}

get_instance_memory() {
    local instance_type=$1
    echo "${INSTANCE_INFO[$instance_type]}" | cut -d: -f6
}

get_instance_notes() {
    local instance_type=$1
    echo "${INSTANCE_INFO[$instance_type]}" | cut -d: -f7
}

# Function to generate instance table
generate_instance_table() {
    echo "  # name         Type  disk  vcpu memory   \$/hr    notes"
    
    # Create array of instance types with their prices for sorting
    local instances_with_prices=()
    local spot_price
    for instance_type in "${!INSTANCE_INFO[@]}"; do
        spot_price=$(get_instance_spot_price "$instance_type")
        instances_with_prices+=("$spot_price:$instance_type")
    done
    
    # Sort by price and print table
    local instance_type arch disk vcpu memory notes
    for entry in $(printf '%s\n' "${instances_with_prices[@]}" | sort -t: -k1 -n); do
        instance_type=$(echo "$entry" | cut -d: -f2)
        arch=$(get_instance_arch "$instance_type")
        spot_price=$(get_instance_spot_price "$instance_type")
        disk=$(get_instance_disk "$instance_type")
        vcpu=$(get_instance_vcpu "$instance_type")
        memory=$(get_instance_memory "$instance_type")
        notes=$(get_instance_notes "$instance_type")
        
        printf "  %-13s  %-5s %-5s %-4s %-8s \$%-6s %s\n" \
            "$instance_type" "$arch" "$disk" "$vcpu" "$memory" "$spot_price" "$notes"
    done
}

# Function to show usage
show_usage() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --instance-type TYPE        EC2 instance type (default: i4g.large)"
    echo "  --firewood-branch BRANCH    Firewood git branch to checkout"
    echo "  --avalanchego-branch BRANCH AvalancheGo git branch to checkout"
    echo "  --coreth-branch BRANCH      Coreth git branch to checkout"
    echo "  --libevm-branch BRANCH      LibEVM git branch to checkout"
    echo "  --nblocks BLOCKS            Number of blocks to download (default: 1m)"
    echo "  --region REGION             AWS region (default: us-west-2)"
    echo "  --spot                      Use spot instance pricing (default depends on instance type)"
    echo "  --dry-run                   Show the aws command that would be run without executing it"
    echo "  --help                      Show this help message"
    echo ""
    echo "Valid instance types:"
    generate_instance_table
    echo ""
    echo "Valid nblocks values: ${VALID_NBLOCKS[*]}"
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --instance-type)
            INSTANCE_TYPE="$2"
            if [[ ! ${INSTANCE_INFO[$INSTANCE_TYPE]+_} ]]; then
                echo "Error: Invalid instance type '$INSTANCE_TYPE'"
                echo "Valid types: ${!INSTANCE_INFO[*]}"
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
        --coreth-branch)
            CORETH_BRANCH="$2"
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

# Set architecture type based on instance type
TYPE=$(get_instance_arch "$INSTANCE_TYPE")

echo "Configuration:"
echo "  Instance Type: $INSTANCE_TYPE ($TYPE)"
echo "  Firewood Branch: ${FIREWOOD_BRANCH:-default}"
echo "  AvalancheGo Branch: ${AVALANCHEGO_BRANCH:-default}"
echo "  Coreth Branch: ${CORETH_BRANCH:-default}"
echo "  LibEVM Branch: ${LIBEVM_BRANCH:-default}"
echo "  Number of Blocks: $NBLOCKS"
echo "  Region: $REGION"
SWAP_SIZE=$(get_instance_swap_size "$INSTANCE_TYPE")
if [ "$SWAP_SIZE" = "0" ]; then
    echo "  Swap: Disabled (sufficient RAM)"
else
    echo "  Swap: $SWAP_SIZE"
fi
if [ "$SPOT_INSTANCE" = true ]; then
    SPOT_PRICE=$(get_instance_spot_price "$INSTANCE_TYPE")
    echo "  Spot Instance: Yes (max price: \$$SPOT_PRICE)"
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
        --region "$REGION" \
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
CORETH_BRANCH_ARG=""
LIBEVM_BRANCH_ARG=""

if [ -n "$FIREWOOD_BRANCH" ]; then
    FIREWOOD_BRANCH_ARG="--branch $FIREWOOD_BRANCH"
fi
if [ -n "$AVALANCHEGO_BRANCH" ]; then
    AVALANCHEGO_BRANCH_ARG="--branch $AVALANCHEGO_BRANCH"
fi
if [ -n "$CORETH_BRANCH" ]; then
    CORETH_BRANCH_ARG="--branch $CORETH_BRANCH"
fi
if [ -n "$LIBEVM_BRANCH" ]; then
    LIBEVM_BRANCH_ARG="--branch $LIBEVM_BRANCH"
fi

# Generate swap configuration based on instance type
SWAP_SIZE=$(get_instance_swap_size "$INSTANCE_TYPE")
if [ "$SWAP_SIZE" = "0" ]; then
    SWAP_CONFIG=""
else
    SWAP_CONFIG="swap:
  filename: /swapfile
  size: $SWAP_SIZE
  maxsize: $SWAP_SIZE"
fi

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
      - ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFuwpEMnsBLdfr7V9SFRTm9XWHEFX3yQQP7nmsFHetBo cardno:26_763_547 brandon
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

__SWAP_CONFIG__

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
  # install task, avalanchego, coreth
  - snap install task --classic
  - >
    sudo -u ubuntu -D /mnt/nvme/ubuntu
    git clone --depth 100 __AVALANCHEGO_BRANCH_ARG__ https://github.com/ava-labs/avalanchego.git
  - >
    sudo -u ubuntu -D /mnt/nvme/ubuntu
    git clone --depth 100 __CORETH_BRANCH_ARG__ https://github.com/ava-labs/coreth.git
  - >
    sudo -u ubuntu -D /mnt/nvme/ubuntu
    git clone --depth 100 __LIBEVM_BRANCH_ARG__ https://github.com/ava-labs/libevm.git
  # force avalanchego to use the checked-out versions of coreth, libevm, and firewood
  - >
    sudo -u ubuntu -D /mnt/nvme/ubuntu/avalanchego
    /usr/local/go/bin/go mod edit -replace
    github.com/ava-labs/firewood-go-ethhash/ffi=../firewood/ffi
  - >
    sudo -u ubuntu -D /mnt/nvme/ubuntu/avalanchego
    /usr/local/go/bin/go mod edit -replace
    github.com/ava-labs/coreth=../coreth
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
  # build avalanchego
  - sudo -u ubuntu --login -D /mnt/nvme/ubuntu/avalanchego go mod tidy
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
    time task reexecute-cchain-range CURRENT_STATE_DIR=/mnt/nvme/ubuntu/exec-data/current-state BLOCK_DIR=/mnt/nvme/ubuntu/exec-data/blocks START_BLOCK=1 END_BLOCK=__END_BLOCK__ CONFIG=firewood METRICS_ENABLED=false
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

# Substitute branch arguments, swap config, and block values in the userdata template
# Use a temporary variable to handle potential newlines in SWAP_CONFIG
TEMP_USERDATA=$(echo "$USERDATA_TEMPLATE" | \
  sed "s|__FIREWOOD_BRANCH_ARG__|$FIREWOOD_BRANCH_ARG|g" | \
  sed "s|__AVALANCHEGO_BRANCH_ARG__|$AVALANCHEGO_BRANCH_ARG|g" | \
  sed "s|__CORETH_BRANCH_ARG__|$CORETH_BRANCH_ARG|g" | \
  sed "s|__LIBEVM_BRANCH_ARG__|$LIBEVM_BRANCH_ARG|g" | \
  sed "s|__NBLOCKS__|$NBLOCKS|g" | \
  sed "s|__END_BLOCK__|$END_BLOCK|g")

# Replace swap config using a different approach to handle multiline content
if [ -n "$SWAP_CONFIG" ]; then
    USERDATA=$(echo "$TEMP_USERDATA" | sed "s|__SWAP_CONFIG__|$SWAP_CONFIG|g" | base64)
else
    USERDATA=$(echo "$TEMP_USERDATA" | sed "s|__SWAP_CONFIG__||g" | base64)
fi
export USERDATA

fi  # End of DRY_RUN=false conditional


SUFFIX=$(hexdump -vn4 -e'4/4 "%08X" 1 "\n"' /dev/urandom)

# Build instance name with branch info
INSTANCE_NAME="$USER-fw-$SUFFIX"
if [ -n "$FIREWOOD_BRANCH" ]; then
    INSTANCE_NAME="$INSTANCE_NAME-fw-$FIREWOOD_BRANCH"
fi
if [ -n "$AVALANCHEGO_BRANCH" ]; then
    INSTANCE_NAME="$INSTANCE_NAME-ag-$AVALANCHEGO_BRANCH"
fi
if [ -n "$CORETH_BRANCH" ]; then
    INSTANCE_NAME="$INSTANCE_NAME-ce-$CORETH_BRANCH"
fi
if [ -n "$LIBEVM_BRANCH" ]; then
    INSTANCE_NAME="$INSTANCE_NAME-le-$LIBEVM_BRANCH"
fi

# Build spot instance market options if requested
SPOT_OPTIONS=""
if [ "$SPOT_INSTANCE" = true ]; then
    MAX_PRICE=$(get_instance_spot_price "$INSTANCE_TYPE")
    SPOT_OPTIONS="--instance-market-options '{\"MarketType\":\"spot\", \"SpotOptions\": {\"MaxPrice\":\"$MAX_PRICE\"}}'"
fi

if [ "$DRY_RUN" = true ]; then
    echo "DRY RUN - Would execute the following command:"
    echo ""
    echo "aws ec2 run-instances \\"
    echo "  --region \"$REGION\" \\"
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
fi

if [ "$DRY_RUN" = false ]; then
    # IP=$(aws ec2 describe-instances --instance-ids "$INSTANCE_ID" --query 'Reservations[].Instances[].PublicIpAddress' --output text)
    set +e

    #IP=$(echo "$JSON" | jq -r '.PublicIpAddress')
    #echo $IP
    #while nc -zv $IP 22; do
        #sleep 1
    #done
fi
