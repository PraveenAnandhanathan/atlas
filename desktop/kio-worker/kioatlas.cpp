/*
 * kioatlas.cpp — KDE KIO worker for ATLAS (T6.5).
 *
 * Implements a KIO::SlaveBase that exposes atlas:// URIs to KDE Plasma
 * applications (Dolphin, KWrite, etc.).  The worker is a child process
 * spawned by klauncher / kioslave5 and communicates via a Unix socket.
 *
 * All filesystem operations delegate to the Rust atlas-gvfs library via
 * the same C-FFI symbols as the GVfs backend (see gvfsbackend-atlas.c).
 *
 * Build (requires KF5):
 *   mkdir build && cd build
 *   cmake -DCMAKE_PREFIX_PATH=$(kf5-config --prefix) ..
 *   make
 *
 * Install:
 *   sudo make install
 *   # writes /usr/lib/qt5/plugins/kf5/kio/atlas.so
 *   # writes /usr/share/kservices5/atlas.protocol
 */

#include <kio/slavebase.h>
#include <kio/global.h>
#include <QUrl>
#include <QByteArray>
#include <QString>
#include <QJsonDocument>
#include <QJsonArray>
#include <QJsonObject>
#include <cstring>

/* C-FFI bridge symbols from libatlas_gvfs.so */
extern "C" {
    int  atlas_gvfs_mount_info(const char *uri, char **out_json);
    void atlas_gvfs_free_string(char *ptr);
    int  atlas_gvfs_enumerate(const char *uri, const char *parent_path, char **out_json);
    int  atlas_gvfs_stat(const char *uri, const char *path, char **out_json);
    int  atlas_gvfs_read_file(const char *uri, const char *path, uint8_t **out_data, size_t *out_len);
    void atlas_gvfs_free_bytes(uint8_t *ptr, size_t len);
    int  atlas_gvfs_write_file(const char *uri, const char *path, const uint8_t *data, size_t len);
    int  atlas_gvfs_delete(const char *uri, const char *path);
    int  atlas_gvfs_make_directory(const char *uri, const char *path);
}

class AtlasProtocol : public KIO::SlaveBase {
public:
    AtlasProtocol(const QByteArray &pool, const QByteArray &app)
        : KIO::SlaveBase("atlas", pool, app) {}
    ~AtlasProtocol() override = default;

private:
    /* Reconstruct the atlas:// mount URI from the KIO URL. */
    static QString mountUri(const QUrl &url) {
        return QString("atlas://%1%2").arg(url.host()).arg(url.path());
    }
    static QString filePath(const QUrl &url) {
        return url.path().isEmpty() ? "/" : url.path();
    }

    UDSEntry entryFromJson(const QJsonObject &obj) {
        UDSEntry entry;
        QString kind = obj.value("kind").toString("File");
        qint64 size  = (qint64)obj.value("size").toDouble(0);
        QString name = obj.value("name").toString();
        entry.fastInsert(KIO::UDSEntry::UDS_NAME, name);
        entry.fastInsert(KIO::UDSEntry::UDS_SIZE, size);
        entry.fastInsert(KIO::UDSEntry::UDS_ACCESS, 0644);
        if (kind == "Dir") {
            entry.fastInsert(KIO::UDSEntry::UDS_FILE_TYPE, S_IFDIR);
        } else {
            entry.fastInsert(KIO::UDSEntry::UDS_FILE_TYPE, S_IFREG);
        }
        return entry;
    }

public:
    void listDir(const QUrl &url) override {
        QString mount = mountUri(url);
        QString path  = filePath(url);
        char *json = nullptr;
        int rc = atlas_gvfs_enumerate(mount.toUtf8(), path.toUtf8(), &json);
        if (rc != 0 || !json) {
            atlas_gvfs_free_string(json);
            error(KIO::ERR_CANNOT_OPEN_FOR_READING, path);
            return;
        }
        QJsonDocument doc = QJsonDocument::fromJson(QByteArray(json));
        atlas_gvfs_free_string(json);
        QJsonArray arr = doc.array();
        totalSize(arr.size());
        for (const QJsonValue &v : arr) {
            listEntry(entryFromJson(v.toObject()));
        }
        finished();
    }

    void stat(const QUrl &url) override {
        QString mount = mountUri(url);
        QString path  = filePath(url);
        char *json = nullptr;
        int rc = atlas_gvfs_stat(mount.toUtf8(), path.toUtf8(), &json);
        if (rc != 0 || !json) {
            atlas_gvfs_free_string(json);
            error(KIO::ERR_DOES_NOT_EXIST, path);
            return;
        }
        QJsonDocument doc = QJsonDocument::fromJson(QByteArray(json));
        atlas_gvfs_free_string(json);
        statEntry(entryFromJson(doc.object()));
        finished();
    }

    void get(const QUrl &url) override {
        QString mount = mountUri(url);
        QString path  = filePath(url);
        uint8_t *data = nullptr;
        size_t len = 0;
        int rc = atlas_gvfs_read_file(mount.toUtf8(), path.toUtf8(), &data, &len);
        if (rc != 0 || !data) {
            atlas_gvfs_free_bytes(data, len);
            error(KIO::ERR_DOES_NOT_EXIST, path);
            return;
        }
        totalSize((KIO::filesize_t)len);
        data(QByteArray(reinterpret_cast<const char *>(data), (int)len));
        atlas_gvfs_free_bytes(data, len);
        finished();
    }

    void put(const QUrl &url, int /*permissions*/, KIO::JobFlags flags) override {
        QString mount = mountUri(url);
        QString path  = filePath(url);
        QByteArray buf;
        while (true) {
            QByteArray chunk;
            dataReq();
            if (readData(chunk) <= 0) break;
            buf.append(chunk);
        }
        int rc = atlas_gvfs_write_file(mount.toUtf8(), path.toUtf8(),
                                        reinterpret_cast<const uint8_t *>(buf.constData()),
                                        (size_t)buf.size());
        if (rc != 0) {
            error(KIO::ERR_CANNOT_WRITE, path);
            return;
        }
        finished();
    }

    void mkdir(const QUrl &url, int /*permissions*/) override {
        QString mount = mountUri(url);
        QString path  = filePath(url);
        int rc = atlas_gvfs_make_directory(mount.toUtf8(), path.toUtf8());
        if (rc != 0) {
            error(KIO::ERR_CANNOT_MKDIR, path);
            return;
        }
        finished();
    }

    void del(const QUrl &url, bool /*isfile*/) override {
        QString mount = mountUri(url);
        QString path  = filePath(url);
        int rc = atlas_gvfs_delete(mount.toUtf8(), path.toUtf8());
        if (rc != 0) {
            error(KIO::ERR_CANNOT_DELETE, path);
            return;
        }
        finished();
    }
};

extern "C" {
    int kdemain(int argc, char **argv) {
        AtlasProtocol slave(argv[2], argv[3]);
        slave.dispatchLoop();
        return 0;
    }
}
