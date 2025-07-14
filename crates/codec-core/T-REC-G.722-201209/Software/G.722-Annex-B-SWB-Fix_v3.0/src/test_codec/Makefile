#
# Makefile for GNU gcc/g++ compiler
#
.SUFFIXES: .o .c .h

SRC_DIR       = ../mainlib
MATH_DIR      = ../basic_math
BASOP_DIR     = ../basicOp_stl2009_v2.3
MDYN_DIR      = ../drcnt

CC	      = gcc
CFLAGS        = -O2 -Wall -fno-builtin
CFLAGS       += -DWMOPS=1
#CFLAGS       += -DDYN_RAM_CNT
CFLAGS       += -DSUPPRESS_COUNTER_RESULTS

CFLAGS       += -I$(SRC_DIR)/pcmswb
CFLAGS       += -I$(MATH_DIR)
CFLAGS       += -I$(BASOP_DIR)
CFLAGS       += -I$(MDYN_DIR)

LD            = $(CC)
LDFLAGS       = -Wall -static

# Add options if needed.
LDFLAGS       +=

LIBS	      = $(SRC_DIR)/libpcmswb.a \
                $(MATH_DIR)/libbasic_math.a -lm \
                $(BASOP_DIR)/libbasop.a -lm \

# List header files.
HDRS	      = $(SRC_DIR)/pcmswb/pcmswb.h

# List C source files.
SRCS          =

OBJS =   $(patsubst %.cpp,%.o,$(SRCS:%.c=%.o))

ENCODER       = encoder

ENCODER_OBJ   = $(ENCODER).o

DECODER       = decoder

DECODER_OBJ   = $(DECODER).o

TARGET        =	$(ENCODER) $(DECODER)

all:		  $(TARGET)

$(ENCODER): $(ENCODER_OBJ) $(OBJS) $(LIBS)
	$(LD) $(LDFLAGS) $(ENCODER_OBJ) $(OBJS) $(LIBS) -o $@

$(DECODER): $(DECODER_OBJ) $(OBJS) $(LIBS)
	$(LD) $(LDFLAGS) $(DECODER_OBJ) $(OBJS) $(LIBS) -o $@

$(ENCODER_OBJ): $(HDRS)
$(DECODER_OBJ): $(HDRS)
$(OBJS): $(HDRS)

clean:
	@rm -f $(TARGET) $(ENCODER_OBJ) $(DECODER_OBJ) $(OBJS) core *.ln

.c.o:
	$(CC) $(CFLAGS) -c $< -o $@
