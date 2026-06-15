/*
 * gvfsbackend-atlas.c — GNOME Virtual File System backend for ATLAS (T6.5).
 *
 * This GObject subclass of GVfsBackend is loaded by gvfsd at runtime from
 * libgvfsbackend-atlas.so.  It translates GIO async job callbacks to
 * synchronous calls into the Rust atlas-gvfs library via the C-FFI symbols
 * exported by libatlas_gvfs.so.
 *
 * Build:
 *   meson setup build && meson compile -C build
 *
 * Install:
 *   sudo meson install -C build
 *   # writes /usr/lib/gvfs/libgvfsbackend-atlas.so
 *   # writes /usr/share/gvfs/mounts/atlas.mount
 *
 * The mount trigger is registered via the D-Bus name
 * "org.gnome.VfsBackend.Atlas" — see atlas.mount.
 */

#include <glib.h>
#include <glib/gi18n.h>
#include <gio/gio.h>

/* GVfs internal headers — available when building against the gvfs source tree. */
#include <daemon/gvfsbackend.h>
#include <daemon/gvfsjobmount.h>
#include <daemon/gvfsjobunmount.h>
#include <daemon/gvfsjobopeniconforread.h>
#include <daemon/gvfsjobread.h>
#include <daemon/gvfsjobwrite.h>
#include <daemon/gvfsjobenumerate.h>
#include <daemon/gvfsjobqueryinfo.h>
#include <daemon/gvfsjobseekread.h>

/* C-FFI bridge symbols from libatlas_gvfs.so */
extern int  atlas_gvfs_mount_info(const char *uri, char **out_json);
extern void atlas_gvfs_free_string(char *ptr);
extern int  atlas_gvfs_enumerate(const char *uri, const char *parent_path,
                                  char **out_json_array);
extern int  atlas_gvfs_stat(const char *uri, const char *path,
                             char **out_json);
extern int  atlas_gvfs_read_file(const char *uri, const char *path,
                                  uint8_t **out_data, size_t *out_len);
extern void atlas_gvfs_free_bytes(uint8_t *ptr, size_t len);
extern int  atlas_gvfs_write_file(const char *uri, const char *path,
                                   const uint8_t *data, size_t len);
extern int  atlas_gvfs_delete(const char *uri, const char *path);
extern int  atlas_gvfs_make_directory(const char *uri, const char *path);

/* ---- GObject boilerplate ------------------------------------------------ */

#define GVFS_TYPE_BACKEND_ATLAS    (gvfs_backend_atlas_get_type())
#define GVFS_BACKEND_ATLAS(obj)    (G_TYPE_CHECK_INSTANCE_CAST((obj), \
                                    GVFS_TYPE_BACKEND_ATLAS, GVfsBackendAtlas))

typedef struct _GVfsBackendAtlas {
    GVfsBackend parent_instance;
    char *mount_uri;   /* e.g. "atlas://mlbox/research/" */
} GVfsBackendAtlas;

typedef struct _GVfsBackendAtlasClass {
    GVfsBackendClass parent_class;
} GVfsBackendAtlasClass;

G_DEFINE_TYPE(GVfsBackendAtlas, gvfs_backend_atlas, G_VFS_TYPE_BACKEND)

/* ---- Helpers ------------------------------------------------------------ */

static void
fill_file_info_from_json(GFileInfo *info, const char *json)
{
    /* Parse JSON produced by atlas_gvfs_stat into GFileInfo attributes.
     * JSON schema: {"path": str, "kind": "File"|"Dir", "size": u64,
     *               "hash": str, "name": str}
     */
    GError *err = NULL;
    JsonParser *parser = json_parser_new();
    if (!json_parser_load_from_data(parser, json, -1, &err)) {
        g_warning("atlas gvfs: stat JSON parse error: %s", err->message);
        g_error_free(err);
        g_object_unref(parser);
        return;
    }
    JsonObject *obj = json_node_get_object(json_parser_get_root(parser));
    const char *kind = json_object_get_string_member_with_default(obj, "kind", "File");
    guint64 size     = (guint64)json_object_get_int_member_with_default(obj, "size", 0);
    const char *name = json_object_get_string_member_with_default(obj, "name", "");

    g_file_info_set_name(info, name);
    g_file_info_set_display_name(info, name);
    g_file_info_set_size(info, (goffset)size);
    if (g_strcmp0(kind, "Dir") == 0) {
        g_file_info_set_file_type(info, G_FILE_TYPE_DIRECTORY);
        g_file_info_set_content_type(info, "inode/directory");
    } else {
        g_file_info_set_file_type(info, G_FILE_TYPE_REGULAR);
    }
    g_object_unref(parser);
}

/* ---- Virtual method implementations ------------------------------------ */

static void
do_mount(GVfsBackend *backend,
         GVfsJobMount *job,
         GMountSpec *mount_spec,
         GMountSource *mount_source,
         gboolean is_automount)
{
    GVfsBackendAtlas *self = GVFS_BACKEND_ATLAS(backend);

    const char *uri = g_mount_spec_get(mount_spec, "uri");
    if (!uri) {
        g_vfs_job_failed(G_VFS_JOB(job), G_IO_ERROR,
                         G_IO_ERROR_INVALID_ARGUMENT,
                         _("ATLAS URI is required (e.g. atlas://host/volume)"));
        return;
    }
    self->mount_uri = g_strdup(uri);

    char *out_json = NULL;
    int rc = atlas_gvfs_mount_info(uri, &out_json);
    atlas_gvfs_free_string(out_json);

    if (rc != 0) {
        g_vfs_job_failed(G_VFS_JOB(job), G_IO_ERROR,
                         G_IO_ERROR_FAILED,
                         _("Failed to connect to ATLAS store at %s"), uri);
        return;
    }

    GMountSpec *spec = g_mount_spec_copy(mount_spec);
    g_vfs_backend_set_mount_spec(backend, spec);
    g_mount_spec_unref(spec);

    g_vfs_backend_set_display_name(backend, self->mount_uri);
    g_vfs_backend_set_icon_name(backend, "folder-remote");
    g_vfs_job_succeeded(G_VFS_JOB(job));
}

static void
do_query_info(GVfsBackend *backend,
              GVfsJobQueryInfo *job,
              const char *filename,
              GFileQueryInfoFlags flags,
              GFileInfo *info,
              GCancellable *cancellable)
{
    GVfsBackendAtlas *self = GVFS_BACKEND_ATLAS(backend);
    char *out_json = NULL;
    int rc = atlas_gvfs_stat(self->mount_uri, filename, &out_json);
    if (rc == 0 && out_json) {
        fill_file_info_from_json(info, out_json);
        atlas_gvfs_free_string(out_json);
        g_vfs_job_succeeded(G_VFS_JOB(job));
    } else {
        atlas_gvfs_free_string(out_json);
        g_vfs_job_failed(G_VFS_JOB(job), G_IO_ERROR,
                         G_IO_ERROR_NOT_FOUND,
                         _("Path not found: %s"), filename);
    }
}

static void
do_enumerate(GVfsBackend *backend,
             GVfsJobEnumerate *job,
             const char *filename,
             GFileAttributeMatcher *attribute_matcher,
             GFileQueryInfoFlags flags,
             GCancellable *cancellable)
{
    GVfsBackendAtlas *self = GVFS_BACKEND_ATLAS(backend);
    char *out_json = NULL;
    int rc = atlas_gvfs_enumerate(self->mount_uri, filename, &out_json);
    if (rc != 0 || !out_json) {
        atlas_gvfs_free_string(out_json);
        g_vfs_job_failed(G_VFS_JOB(job), G_IO_ERROR,
                         G_IO_ERROR_NOT_FOUND,
                         _("Cannot enumerate %s"), filename);
        return;
    }

    /* Parse JSON array of entries and emit them one by one. */
    GError *err = NULL;
    JsonParser *parser = json_parser_new();
    if (json_parser_load_from_data(parser, out_json, -1, &err)) {
        JsonArray *arr = json_node_get_array(json_parser_get_root(parser));
        guint n = json_array_get_length(arr);
        for (guint i = 0; i < n; i++) {
            JsonObject *obj = json_array_get_object_element(arr, i);
            GFileInfo *info = g_file_info_new();
            char *entry_json = json_to_string(json_node_new_from_object(obj), FALSE);
            fill_file_info_from_json(info, entry_json);
            g_free(entry_json);
            g_vfs_job_enumerate_add_info(job, info);
            g_object_unref(info);
        }
    } else {
        g_warning("atlas gvfs: enumerate JSON parse error: %s", err->message);
        g_error_free(err);
    }
    g_object_unref(parser);
    atlas_gvfs_free_string(out_json);

    g_vfs_job_enumerate_done(job);
    g_vfs_job_succeeded(G_VFS_JOB(job));
}

static void
do_open_for_read(GVfsBackend *backend,
                 GVfsJobOpenForRead *job,
                 const char *filename,
                 GCancellable *cancellable)
{
    /* Fetch entire file content up front; store pointer as handle. */
    GVfsBackendAtlas *self = GVFS_BACKEND_ATLAS(backend);
    uint8_t *data = NULL;
    size_t len = 0;
    int rc = atlas_gvfs_read_file(self->mount_uri, filename, &data, &len);
    if (rc != 0) {
        g_vfs_job_failed(G_VFS_JOB(job), G_IO_ERROR,
                         G_IO_ERROR_NOT_FOUND, _("File not found: %s"), filename);
        return;
    }

    /* Store (data, len, offset) as a three-word struct in the handle. */
    typedef struct { uint8_t *data; size_t len; size_t pos; } ReadHandle;
    ReadHandle *h = g_new(ReadHandle, 1);
    h->data = data; h->len = len; h->pos = 0;
    g_vfs_job_open_for_read_set_handle(job, h);
    g_vfs_job_succeeded(G_VFS_JOB(job));
}

static void
do_read(GVfsBackend *backend,
        GVfsJobRead *job,
        GVfsBackendHandle handle,
        char *buffer,
        gsize bytes_requested,
        GCancellable *cancellable)
{
    typedef struct { uint8_t *data; size_t len; size_t pos; } ReadHandle;
    ReadHandle *h = (ReadHandle *)handle;
    gsize available = h->len - h->pos;
    gsize to_copy = MIN(bytes_requested, available);
    if (to_copy > 0) {
        memcpy(buffer, h->data + h->pos, to_copy);
        h->pos += to_copy;
    }
    g_vfs_job_read_set_size(job, to_copy);
    g_vfs_job_succeeded(G_VFS_JOB(job));
}

static void
do_close_read(GVfsBackend *backend,
              GVfsJobCloseRead *job,
              GVfsBackendHandle handle,
              GCancellable *cancellable)
{
    typedef struct { uint8_t *data; size_t len; size_t pos; } ReadHandle;
    ReadHandle *h = (ReadHandle *)handle;
    atlas_gvfs_free_bytes(h->data, h->len);
    g_free(h);
    g_vfs_job_succeeded(G_VFS_JOB(job));
}

static void
do_make_directory(GVfsBackend *backend,
                  GVfsJobMakeDirectory *job,
                  const char *filename,
                  GCancellable *cancellable)
{
    GVfsBackendAtlas *self = GVFS_BACKEND_ATLAS(backend);
    int rc = atlas_gvfs_make_directory(self->mount_uri, filename);
    if (rc == 0) {
        g_vfs_job_succeeded(G_VFS_JOB(job));
    } else {
        g_vfs_job_failed(G_VFS_JOB(job), G_IO_ERROR,
                         G_IO_ERROR_FAILED, _("mkdir failed: %s"), filename);
    }
}

static void
do_delete(GVfsBackend *backend,
          GVfsJobDelete *job,
          const char *filename,
          GCancellable *cancellable)
{
    GVfsBackendAtlas *self = GVFS_BACKEND_ATLAS(backend);
    int rc = atlas_gvfs_delete(self->mount_uri, filename);
    if (rc == 0) {
        g_vfs_job_succeeded(G_VFS_JOB(job));
    } else {
        g_vfs_job_failed(G_VFS_JOB(job), G_IO_ERROR,
                         G_IO_ERROR_FAILED, _("delete failed: %s"), filename);
    }
}

/* ---- GObject init / finalise ------------------------------------------- */

static void
gvfs_backend_atlas_finalize(GObject *object)
{
    GVfsBackendAtlas *self = GVFS_BACKEND_ATLAS(object);
    g_free(self->mount_uri);
    G_OBJECT_CLASS(gvfs_backend_atlas_parent_class)->finalize(object);
}

static void
gvfs_backend_atlas_class_init(GVfsBackendAtlasClass *klass)
{
    GObjectClass *gobject_class = G_OBJECT_CLASS(klass);
    GVfsBackendClass *backend_class = G_VFS_BACKEND_CLASS(klass);

    gobject_class->finalize    = gvfs_backend_atlas_finalize;
    backend_class->mount        = do_mount;
    backend_class->query_info   = do_query_info;
    backend_class->enumerate    = do_enumerate;
    backend_class->open_for_read = do_open_for_read;
    backend_class->read         = do_read;
    backend_class->close_read   = do_close_read;
    backend_class->make_directory = do_make_directory;
    backend_class->delete       = do_delete;
}

static void
gvfs_backend_atlas_init(GVfsBackendAtlas *self)
{
    self->mount_uri = NULL;
}

/* ---- Plugin entry point ------------------------------------------------- */

/* GVfs backend discovery: gvfsd looks for g_vfs_backend_get_type symbols
 * in each shared library under $(libdir)/gvfs/. */
GType
g_vfs_backend_get_type(void)
{
    return GVFS_TYPE_BACKEND_ATLAS;
}
