#!/bin/bash
set -o errexit

# install the keyrings needed to validate the grafana apt repository
if ! [ -d /etc/apt/keyrings ]; then
  mkdir -p /etc/apt/keyrings/
fi
if ! [ -f /etc/apt/keyrings/grafana.gpg ]; then
  wget -q -O - https://apt.grafana.com/gpg.key | gpg --dearmor | sudo tee /etc/apt/keyrings/grafana.gpg > /dev/null
  echo "deb [signed-by=/etc/apt/keyrings/grafana.gpg] https://apt.grafana.com stable main" | sudo tee -a /etc/apt/sources.list.d/grafana.list
fi
apt-get update

# set up the systemd configuration to allow grafana to bind to port 80
if ! [ -d /etc/systemd/system/grafana-server.service.d ]; then
  mkdir -p /etc/systemd/system/grafana-server.service.d
fi

if ! [ -f /etc/systemd/system/grafana-server.service.d/override.conf ]; then
  cat > /etc/systemd/system/grafana-server.service.d/override.conf <<!
[Service]
# Give the CAP_NET_BIND_SERVICE capability
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
AmbientCapabilities=CAP_NET_BIND_SERVICE
  
# A private user cannot have process capabilities on the host's user
# namespace and thus CAP_NET_BIND_SERVICE has no effect.
PrivateUsers=false
!
fi

# install the grafana and prometheus packages if they are not already installed
pkgs=(grafana prometheus)
install_pkgs=()
for pkg in "${pkgs[@]}"; do
  if ! dpkg -s "$pkg" > /dev/null 2>&1; then
    install_pkgs+=("$pkg")
  fi
done
if [ "${#install_pkgs[@]}" -gt 0 ]; then
  apt-get install -y "${install_pkgs[@]}"
fi

# configure grafana to listen on port 80
if ! grep -q '^http_port = 80$' /etc/grafana/grafana.ini; then
  perl -pi -e 's/^;?http_port = .*/http_port = 80/' /etc/grafana/grafana.ini
fi

sed -i -E "s|^;?\s*admin_user\s*=.*|admin_user = admin|" /etc/grafana/grafana.ini
sed -i -E "s|^;?\s*admin_password\s*=.*|admin_password = firewoodisfast|" /etc/grafana/grafana.ini

{
  echo 'apiVersion: 1'
  echo 'datasources:'
  echo '  - name: Prometheus'
  echo '    type: prometheus'
  echo '    access: proxy'
  echo '    orgId: 1'
  echo '    url: http://localhost:9090'
  echo '    isDefault: true'
  echo '    editable: true'
} | sudo tee /etc/grafana/provisioning/datasources/prometheus.yml >/dev/null

{
  echo 'apiVersion: 1'
  echo 'providers:'
  echo "  - name: 'files'"
  echo '    orgId: 1'
  echo "    folder: 'Provisioned'"
  echo '    type: file'
  echo '    disableDeletion: false'
  echo '    editable: true'
  echo '    options:'
  echo '      path: /var/lib/grafana/dashboards'
} | sudo tee /etc/grafana/provisioning/dashboards/dashboards.yaml >/dev/null

sudo mkdir -p /var/lib/grafana/dashboards
sudo wget -O /var/lib/grafana/dashboards/firewood.json https://github.com/ava-labs/firewood/raw/refs/heads/main/benchmark/Grafana-dashboard.json 

# configure prometheus to scrape firewood
if ! grep -q '^  - job_name: firewood$' /etc/prometheus/prometheus.yml; then
  cat >> /etc/prometheus/prometheus.yml <<!
  - job_name: firewood
    static_configs:
      - targets: ['localhost:3000']
  - job_name: coreth
    metrics_path: /debug/metrics/prometheus
    static_configs:
      - targets: ['localhost:6060']
  - job_name: avalanchego
    metrics_path: /ext/metrics
    static_configs:
      - targets: ['localhost:10023']
!
fi

# configure the node exporter to use all available filesystems
if ! grep -q collector.filesystem.mount-points-exclude /etc/default/prometheus-node-exporter; then
  cat >> /etc/default/prometheus-node-exporter <<!
ARGS="--collector.filesystem.mount-points-exclude=\"^/(dev|proc|run|sys|media|var/lib/docker/.+)($|/)\""
!
fi

# restart the grafana and prometheus services
# it's okay if the grafana service is not running
killall grafana-server || true
systemctl daemon-reload
systemctl enable grafana-server
systemctl start grafana-server
systemctl restart prometheus
