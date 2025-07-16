CC ?= gcc
CFLAGS ?= -O2 -Wall -std=c99
LDFLAGS ?= -laio

# Check for liburing availability
LIBURING_AVAILABLE := $(shell pkg-config --exists liburing 2>/dev/null && echo yes || echo no)
ifeq ($(LIBURING_AVAILABLE),yes)
    CFLAGS += -DHAVE_LIBURING
    LDFLAGS += -luring
endif

# Source files
SOURCES = main.c io_operations.c
HEADERS = disk_warmer.h
TARGET = disk-warmer

# Object files
OBJECTS = $(SOURCES:.c=.o)

# Default target
all: $(TARGET)

# Build the main target
$(TARGET): $(OBJECTS)
	$(CC) $(OBJECTS) -o $(TARGET) $(LDFLAGS)

# Build object files
%.o: %.c $(HEADERS)
	$(CC) $(CFLAGS) -c $< -o $@

# Install target
install: $(TARGET)
	install -d $(DESTDIR)/usr/local/bin
	install -m 755 $(TARGET) $(DESTDIR)/usr/local/bin/

# Clean target
clean:
	rm -f $(TARGET) $(OBJECTS) disk-warmer-*
	rm -f cppcheck-report.xml

# Build variants
portable:
	$(MAKE) CC=gcc CFLAGS="-O2 -Wall -std=c99" TARGET=disk-warmer-portable

# Static build for maximum compatibility
static:
	$(MAKE) CC=gcc CFLAGS="-O2 -Wall -std=c99" LDFLAGS="-static -laio" TARGET=disk-warmer-static

# Static build with liburing (if available)
static-full:
ifeq ($(LIBURING_AVAILABLE),yes)
	$(MAKE) CC=gcc CFLAGS="-O2 -Wall -std=c99 -DHAVE_LIBURING" LDFLAGS="-static -laio -luring" TARGET=disk-warmer-static-full
else
	$(MAKE) CC=gcc CFLAGS="-O2 -Wall -std=c99" LDFLAGS="-static -laio" TARGET=disk-warmer-static
endif

# Development build with debug symbols
debug:
	$(MAKE) CFLAGS="-O0 -g -Wall -std=c99 -DDEBUG" TARGET=disk-warmer-debug

# Sanitizer builds for runtime analysis
sanitize-address:
	$(MAKE) CC=gcc CFLAGS="-O1 -g -fsanitize=address -fno-omit-frame-pointer -Wall -std=c99" \
		LDFLAGS="-fsanitize=address -laio" TARGET=disk-warmer-asan

sanitize-undefined:
	$(MAKE) CC=gcc CFLAGS="-O1 -g -fsanitize=undefined -fno-omit-frame-pointer -Wall -std=c99" \
		LDFLAGS="-fsanitize=undefined -laio" TARGET=disk-warmer-ubsan

# All sanitizers (for comprehensive testing)
sanitize-all: sanitize-address sanitize-undefined
	@echo "=== All sanitizer builds complete ==="
	@echo "🧪 Available sanitized binaries:"
	@echo "  - disk-warmer-asan (AddressSanitizer)"
	@echo "  - disk-warmer-ubsan (UndefinedBehaviorSanitizer)"

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
	@echo "  all         - Build disk-warmer (default)"
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

.PHONY: all install clean portable static static-full debug \
        analyze analyze-cppcheck analyze-clang-tidy \
        sanitize-all sanitize-address sanitize-undefined \
        help 