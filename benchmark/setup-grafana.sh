#!/bin/bash
set -o errexit

# install the keyrings needed to validate the grafana apt repository
mkdir -p /etc/apt/keyrings/
wget -q -O - https://apt.grafana.com/gpg.key | gpg --dearmor | sudo tee /etc/apt/keyrings/grafana.gpg > /dev/null
echo "deb [signed-by=/etc/apt/keyrings/grafana.gpg] https://apt.grafana.com stable main" | sudo tee -a /etc/apt/sources.list.d/grafana.list
apt-get update

# set up the systemd configuration to allow grafana to bind to port 80
mkdir -p /etc/systemd/system/grafana-server.service.d
cat > /etc/systemd/system/grafana-server.service.d/override.conf <<!
[Service]
# Give the CAP_NET_BIND_SERVICE capability
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
AmbientCapabilities=CAP_NET_BIND_SERVICE
  
# A private user cannot have process capabilities on the host's user
# namespace and thus CAP_NET_BIND_SERVICE has no effect.
PrivateUsers=false
!

# install the grafana and prometheus packages
apt-get install -y grafana prometheus

# configure grafana to listen on port 80
perl -pi -e 's/^;?http_port = .*/http_port = 80/' /etc/grafana/grafana.ini

# configure prometheus to scrape firewood
cat >> /etc/prometheus/prometheus.yml <<!
  - job_name: firewood
    static_configs:
      - targets: ['localhost:3000']
  - job_name: geth
    static_configs:
      - targets: ['localhost:6060']
!

# configure the node exporter to use all available filesystems
cat >> /etc/default/prometheus-node-exporter <<!
ARGS="--collector.filesystem.mount-points-exclude=\"^/(dev|proc|run|sys|media|var/lib/docker/.+)($|/)\""
!

# restart the grafana and prometheus services
killall grafana-server
systemctl daemon-reload
systemctl enable grafana-server
systemctl start grafana-server
systemctl restart prometheus
