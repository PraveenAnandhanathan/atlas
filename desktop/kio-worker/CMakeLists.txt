cmake_minimum_required(VERSION 3.16)
project(kioatlas VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_package(ECM REQUIRED NO_MODULE)
set(CMAKE_MODULE_PATH ${ECM_MODULE_PATH})

include(KDEInstallDirs)
include(KDECMakeSettings)
include(KDECompilerSettings)

find_package(KF5 REQUIRED COMPONENTS KIO)
find_package(Qt5 REQUIRED COMPONENTS Core)
find_package(PkgConfig REQUIRED)
pkg_check_modules(ATLAS_GVFS REQUIRED atlas-gvfs)

add_library(kioatlas MODULE kioatlas.cpp)
target_link_libraries(kioatlas
    KF5::KIOCore
    Qt5::Core
    ${ATLAS_GVFS_LIBRARIES}
)
target_include_directories(kioatlas PRIVATE ${ATLAS_GVFS_INCLUDE_DIRS})

install(TARGETS kioatlas DESTINATION ${KDE_INSTALL_PLUGINDIR}/kf5/kio)
install(FILES atlas.protocol DESTINATION ${KDE_INSTALL_DATADIR}/kservices5)
