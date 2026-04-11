package dashboards

import (
	"fmt"

	v2 "github.com/grafana/grafana-foundation-sdk/go/dashboardv2beta1"
)

// BuildOptions controls optional overrides applied during dashboard generation.
type BuildOptions struct {
	// TitleOverride replaces the dashboard's default title when non-empty.
	TitleOverride string
}

// Dashboard is the interface that every dashboard definition must satisfy.
type Dashboard interface {
	Name() string
	Description() string
	Build(opts BuildOptions) (v2.Dashboard, error)
}

// Registry holds all registered dashboards and provides lookup by name.
type Registry struct {
	entries []Dashboard
	byName  map[string]Dashboard
}

// NewRegistry returns an empty Registry.
func NewRegistry() *Registry {
	return &Registry{
		entries: make([]Dashboard, 0, 4),
		byName:  make(map[string]Dashboard),
	}
}

// Register adds a Dashboard to the registry. Panics on duplicate names.
func (r *Registry) Register(d Dashboard) {
	if _, exists := r.byName[d.Name()]; exists {
		panic(fmt.Sprintf("dashboard %q already registered", d.Name()))
	}
	r.entries = append(r.entries, d)
	r.byName[d.Name()] = d
}

// All returns all registered dashboards in registration order.
func (r *Registry) All() []Dashboard {
	return r.entries
}

// Get returns the dashboard with the given name, or false if not found.
func (r *Registry) Get(name string) (Dashboard, bool) {
	d, ok := r.byName[name]
	return d, ok
}
