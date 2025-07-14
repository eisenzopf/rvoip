#
# Makefile for GNU gcc/g++ compiler
#
.SUFFIXES: .o .c .h

CC	          = gcc
CFLAGS	      = -O2 -Wall -fno-builtin

# Add options if needed.
CFLAGS        += -DWMOPS=1
CFLAGS        += -I../basicOp_stl2009_v2.3 -I../mainlib/util

CXX           = g++
CXXFLAGS      = $(CFLAGS)

# Do not change the library name.
LIBRARY	      = libbasic_math.a

# List header files.
HDRS	      = log2.h \
		math_op.h   \
		oper_32b.h

# List C source files.
SRCS	      = log2.c \
		math_op.c   \
		oper_32b.c

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
