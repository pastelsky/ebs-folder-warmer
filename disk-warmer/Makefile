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

# Build variants
portable:
	$(MAKE) CC=gcc CFLAGS="-O2 -Wall -std=c99" TARGET=disk-warmer-portable

# Development build with debug symbols
debug:
	$(MAKE) CFLAGS="-O0 -g -Wall -std=c99 -DDEBUG" TARGET=disk-warmer-debug

# Static analysis
analyze: $(SOURCES) $(HEADERS)
	@echo "Running static analysis..."
	@which cppcheck > /dev/null 2>&1 && cppcheck --enable=all --std=c99 $(SOURCES) || echo "cppcheck not found, skipping analysis"

# Help
help:
	@echo "Available targets:"
	@echo "  all       - Build disk-warmer (default)"
	@echo "  debug     - Build with debug symbols"
	@echo "  install   - Install to /usr/local/bin"
	@echo "  clean     - Remove built files"
	@echo "  portable  - Build portable binary"
	@echo "  analyze   - Run static analysis (requires cppcheck)"
	@echo "  help      - Show this help"
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

.PHONY: all install clean portable debug analyze help 