CC ?= gcc
CFLAGS ?= -O2 -Wall -std=c99
LDFLAGS ?= -lpthread

# Check for Linux to add specific libraries
ifeq ($(OS),Linux)
    LDFLAGS += -laio
    # Check for liburing availability
    LIBURING_AVAILABLE := $(shell pkg-config --exists liburing 2>/dev/null && echo yes || echo no)
    ifeq ($(LIBURING_AVAILABLE),yes)
        CFLAGS += -DHAVE_LIBURING
        LDFLAGS += -luring
    endif
endif
OS := $(shell uname)

# Source files
SOURCES = main.c io_operations.c filesystem.c
HEADERS = page_cache_warmer.h
TARGET = page-cache-warmer

# Object files
OBJECTS = main.o io_operations.o filesystem.o

# Default target
all: $(TARGET)

# Build the main target
$(TARGET): $(OBJECTS)
	$(CC) $(OBJECTS) -o $(TARGET) $(LDFLAGS)

# Build object files
%.o: %.c $(HEADERS)
	$(CC) $(CFLAGS) -c $< -o $@

filesystem.o: filesystem.c page_cache_warmer.h
	$(CC) $(CFLAGS) -c filesystem.c

# Install target
install: $(TARGET)
	install -d $(DESTDIR)/usr/local/bin
	install -m 755 $(TARGET) $(DESTDIR)/usr/local/bin/

# Clean target
clean:
	rm -f $(TARGET) $(OBJECTS) page-cache-warmer-*
	rm -f cppcheck-report.xml

# Debug build
debug:
	$(MAKE) CFLAGS="-O0 -g -Wall -std=c99 -DDEBUG" TARGET=page-cache-warmer-debug

# Sanitizer builds
sanitize-address:
	$(MAKE) CC=gcc CFLAGS="-O1 -g -fsanitize=address -fno-omit-frame-pointer -Wall -std=c99" \
		LDFLAGS="-fsanitize=address -laio -lpthread" TARGET=page-cache-warmer-asan

sanitize-undefined:
	$(MAKE) CC=gcc CFLAGS="-O1 -g -fsanitize=undefined -fno-omit-frame-pointer -Wall -std=c99" \
		LDFLAGS="-fsanitize=undefined -laio" TARGET=page-cache-warmer-ubsan

# All sanitizers (for comprehensive testing)
sanitize-all: sanitize-address sanitize-undefined
	@echo "=== All sanitizer builds complete ==="
	@echo "🧪 Available sanitized binaries:"
	@echo "  - page-cache-warmer-asan (AddressSanitizer)"
	@echo "  - page-cache-warmer-ubsan (UndefinedBehaviorSanitizer)"

# Static Analysis and Linting
CPPCHECK_OPTS = --enable=all --std=c99 --platform=unix64 --suppressions-list=cppcheck-suppressions.txt --inline-suppr --quiet
CLANG_TIDY_OPTS = -checks=-*,readability-*,performance-*,portability-*,bugprone-*,clang-analyzer-*,cert-*

# Individual analysis targets
analyze-cppcheck: $(SOURCES) $(HEADERS)
	@echo "=== Running cppcheck static analysis ==="
	@which cppcheck > /dev/null 2>&1 && \
		cppcheck $(CPPCHECK_OPTS) --xml --xml-version=2 $(SOURCES) 2> cppcheck-report.xml && \
		cppcheck $(CPPCHECK_OPTS) $(SOURCES) || \
		echo "❌ cppcheck not found. Install with: sudo apt install cppcheck"

analyze-clang-tidy: $(SOURCES) $(HEADERS)
	@echo "=== Running clang-tidy analysis ==="
	@which clang-tidy > /dev/null 2>&1 && \
		clang-tidy $(CLANG_TIDY_OPTS) $(SOURCES) -- $(CFLAGS) -I. || \
		echo "❌ clang-tidy not found. Install with: sudo apt install clang-tidy"

# Comprehensive analysis
analyze: analyze-cppcheck analyze-clang-tidy
	@echo "=== Static analysis complete ==="
	@echo "📊 Check generated reports:"
	@echo "  - cppcheck-report.xml"

# Help
help:
	@echo "Available targets:"
	@echo ""
	@echo "📦 Build Targets:"
	@echo "  all         - Build page-cache-warmer (default)"
	@echo "  debug       - Build with debug symbols"
	@echo "  portable    - Build portable binary"
	@echo "  static      - Build statically linked binary (maximum compatibility)"
	@echo "  static-full - Build static binary with all features (liburing + libaio)"
	@echo ""
	@echo "🧪 Analysis & Testing:"
	@echo "  analyze           - Run static analysis (cppcheck + clang-tidy)"
	@echo "  analyze-cppcheck  - Run cppcheck only"
	@echo "  analyze-clang-tidy - Run clang-tidy only"
	@echo ""
	@echo "🛡️  Sanitizer Builds:"
	@echo "  sanitize-all      - Build essential sanitizer variants"
	@echo "  sanitize-address  - Build with AddressSanitizer"
	@echo "  sanitize-undefined - Build with UBSanitizer"
	@echo ""
	@echo "🔧 Utilities:"
	@echo "  install     - Install to /usr/local/bin"
	@echo "  clean       - Remove built files"
	@echo "  help        - Show this help"
	@echo ""
	@echo "Performance Features:"
ifeq ($(LIBURING_AVAILABLE),yes)
	@echo "  ✓ io_uring - High-performance async I/O (Linux 5.1+)"
else
	@echo "  ✗ io_uring - Not available (install liburing-dev for best performance)"
endif
	@echo "  ✓ O_DIRECT - Direct I/O bypassing page cache"
	@echo "  ✓ Auto-alignment - Device sector size detection"
	@echo "  ✓ Modular design - Separated concerns for maintainability"
	@echo ""
	@echo "Variables:"
	@echo "  CC        - C compiler (default: gcc)"
	@echo "  CFLAGS    - Compiler flags (default: -O2 -Wall -std=c99)"
	@echo "  LDFLAGS   - Linker flags (default: -laio, +luring if available)"
	@echo "  DESTDIR   - Install destination prefix"

test: $(TARGET)
	cd tests && ./e2e_warming.sh

.PHONY: all install clean portable static static-full debug \
        analyze analyze-cppcheck analyze-clang-tidy \
        sanitize-all sanitize-address sanitize-undefined \
        help 