#
# Makefile for GNU gcc/g++ compiler
#
.SUFFIXES: .o .c .cpp .h

CC	          = gcc
CFLAGS	      = -O2 -Wall

# Add options if needed.
CFLAGS        += -DWMOPS=1
CFLAGS        += -fno-builtin

CXX           = g++
CXXFLAGS      = $(CFLAGS)

# Do not change the library name.
LIBRARY	      = libbasop.a

# List header files.
HDRS	      = basop32.h \
				control.h \
				count.h   \
				enh1632.h \
				move.h    \
				patch.h   \
				stl.h     \
				typedef.h \
				typedefs.h
#				enh40.h

# List C source files.
SRCS	      = basop32.c \
				control.c \
				count.c   \
				enh1632.c
#				enh40.c

OBJS =   $(patsubst %.cpp,%.o,$(SRCS:%.c=%.o))

all:		$(LIBRARY)

$(LIBRARY):	$(OBJS)
		@echo  "Loading $(LIBRARY) ..."
		ar cru $(LIBRARY) $(OBJS)
		ranlib $(LIBRARY)

$(OBJS): $(HDRS)

clean:;		@rm -f $(LIBRARY) $(OBJS) core *.ln

.c.o:
	$(CC) $(CFLAGS) -c $< -o $@
.cpp.o:
	$(CXX) $(CXXFLAGS) -c $< -o $@
