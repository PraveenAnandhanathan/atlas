project('gvfsbackend-atlas', 'c',
  version: '0.1.0',
  license: 'MIT',
)

gvfs_dep  = dependency('gvfs', version: '>= 1.48')
glib_dep  = dependency('glib-2.0')
gio_dep   = dependency('gio-2.0')
json_dep  = dependency('json-glib-1.0')
atlas_dep = dependency('atlas-gvfs', fallback: ['atlas-gvfs', 'atlas_gvfs_dep'])

gvfs_backend_dir = gvfs_dep.get_variable(pkgconfig: 'backenddir',
                                         default_value: '/usr/lib/gvfs')

shared_library(
  'gvfsbackend-atlas',
  sources: ['gvfsbackend-atlas.c'],
  dependencies: [gvfs_dep, glib_dep, gio_dep, json_dep, atlas_dep],
  install: true,
  install_dir: gvfs_backend_dir,
  name_prefix: '',
)

install_data(
  'atlas.mount',
  install_dir: join_paths(get_option('datadir'), 'gvfs', 'mounts'),
)
