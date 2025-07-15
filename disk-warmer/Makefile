CC ?= gcc
CFLAGS ?= -O2 -Wall -std=c99
LDFLAGS ?= -laio

# Source and target
SRC = main.c
TARGET = disk-warmer

# Default target
all: $(TARGET)

# Build the main target
$(TARGET): $(SRC)
	$(CC) $(CFLAGS) -o $(TARGET) $(SRC) $(LDFLAGS)

# Install target
install: $(TARGET)
	install -d $(DESTDIR)/usr/local/bin
	install -m 755 $(TARGET) $(DESTDIR)/usr/local/bin/

# Clean target
clean:
	rm -f $(TARGET)

# Cross-compilation helpers
arm64: 
	$(MAKE) CC=aarch64-linux-gnu-gcc TARGET=disk-warmer-arm64

static:
	$(MAKE) CC=musl-gcc LDFLAGS="-static -laio" TARGET=disk-warmer-static

# Help
help:
	@echo "Available targets:"
	@echo "  all       - Build disk-warmer (default)"
	@echo "  install   - Install to /usr/local/bin"
	@echo "  clean     - Remove built files"
	@echo "  arm64     - Cross-compile for ARM64"
	@echo "  static    - Build statically linked binary"
	@echo "  help      - Show this help"
	@echo ""
	@echo "Variables:"
	@echo "  CC        - C compiler (default: gcc)"
	@echo "  CFLAGS    - Compiler flags (default: -O2 -Wall -std=c99)"
	@echo "  LDFLAGS   - Linker flags (default: -laio)"
	@echo "  DESTDIR   - Install destination prefix"

.PHONY: all install clean arm64 static help 