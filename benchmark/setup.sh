#!/bin/bash
set -o errexit
#### run these commands from the root user ####

"$(dirname $0)"/setup-grafana.sh

apt upgrade -y

apt install -y git protobuf-compiler build-essential apt-transport-https net-tools zfsutils-linux
  
perl -pi -e 's/^;?http_port = .*/http_port = 80/' /etc/grafana/grafana.ini
cat >> /etc/prometheus/prometheus.yml <<!
  - job_name: firewood
    static_configs:
      - targets: ['localhost:3000']
!
cat >> /etc/default/prometheus-node-exporter <<!
ARGS="--collector.filesystem.mount-points-exclude=\"^/(dev|proc|run|sys|media|var/lib/docker/.+)($|/)\""
!

systemctl daemon-reload
killall grafana-server
systemctl start grafana-server
systemctl enable grafana-server.service
systemctl restart prometheus
  
NVME_DEV="$(realpath /dev/disk/by-id/nvme-Amazon_EC2_NVMe_Instance_Storage_* | uniq)"
if [ -n "$NVME_DEV" ]; then
  mkfs.ext4 -E nodiscard -i 6291456 "$NVME_DEV"
  NVME_MOUNT=/mnt/nvme
  mkdir -p "$NVME_MOUNT"
  mount -o noatime "$NVME_DEV" "$NVME_MOUNT"
  echo "$NVME_DEV $NVME_MOUNT ext4 noatime 0 0" >> /etc/fstab
  mkdir -p "$NVME_MOUNT/ubuntu/firewood"
  chown ubuntu:ubuntu "$NVME_MOUNT/ubuntu" "$NVME_MOUNT/ubuntu/firewood"
  ln -s "$NVME_MOUNT/ubuntu/firewood" /home/ubuntu/firewood
fi


#### you can switch to the ubuntu user here ####
  
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"
cd firewood
git clone https://github.com/ava-labs/firewood.git .
git checkout rkuris/prometheus
cargo build --profile maxperf

#### stop here, these commands are run by hand ####

# 10M rows:
nohup time cargo run --profile maxperf --bin benchmark -- -n 1000 create &
nohup time cargo run --profile maxperf --bin benchmark -- -n 1000 zipf &
nohup time cargo run --profile maxperf --bin benchmark -- -n 1000 single &

# 50M rows:
nohup time cargo run --profile maxperf --bin benchmark -- -n 5000 create &
nohup time cargo run --profile maxperf --bin benchmark -- -n 5000 zipf &
nohup time cargo run --profile maxperf --bin benchmark -- -n 5000 single &

# 100M rows:
nohup time cargo run --profile maxperf --bin benchmark -- -n 10000 create &
nohup time cargo run --profile maxperf --bin benchmark -- -n 10000 zipf &
nohup time cargo run --profile maxperf --bin benchmark -- -n 10000 single &
