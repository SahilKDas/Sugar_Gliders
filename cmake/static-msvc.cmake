# Rust's +crt-static must also apply to the C codec: a static .lib can otherwise
# silently import the dynamically linked Visual C++ runtime.
if("$ENV{CARGO_CFG_TARGET_ENV}" STREQUAL "msvc")
    set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>" CACHE STRING "Static CRT" FORCE)
    foreach(language C CXX)
        set(CMAKE_${language}_FLAGS_RELEASE "/MT /O2 /Ob2 /DNDEBUG" CACHE STRING "Static CRT release" FORCE)
        set(CMAKE_${language}_FLAGS_RELWITHDEBINFO "/MT /O2 /Zi /DNDEBUG" CACHE STRING "Static CRT with symbols" FORCE)
        set(CMAKE_${language}_FLAGS_MINSIZEREL "/MT /O1 /Ob1 /DNDEBUG" CACHE STRING "Static CRT small" FORCE)
        set(CMAKE_${language}_FLAGS_DEBUG "/MTd /Zi /Ob0 /Od /RTC1" CACHE STRING "Static CRT debug" FORCE)
    endforeach()
endif()
