#
# Makefile for GNU gcc/g++ compiler
#
.SUFFIXES: .o .c .h

CC	      = gcc
CFLAGS	      = -O2 -fno-builtin
CFLAGS	      = -O2 -Wall -fno-builtin

CFLAGS        += -DWMOPS=1
#CFLAGS        += -DDYN_RAM_CNT
#CFLAGS        += -DMEM_STT

CFLAGS        += -I../basic_math
CFLAGS        += -I../basicOp_stl2009_v2.3
CFLAGS        += -I../drcnt
CFLAGS        += -Iavq -Iutil -Ipcmswb -Ins -Ig722 -Ibwe

# Do not change the library name.
LIBRARY	      = libpcmswb.a

# List common header files.
HDRS	      = 

# List C source files.
SRC_NSCORE = ns/lpctool.c \
		ns/autocorr_ns.c \
		ns/table_lowband.c \

SRC_G722CORE  = g722/funcg722.c \
		g722/g722.c \
		g722/g722_plc.c \
		g722/g722_tables.c \
		g722/g722_plc_tables.c \
		g722/lsbcod_ns.c \
		g722/hsb_enh.c \
		g722/ns_common.c \

SRC_BWE = bwe/bwe_enc.c \
		bwe/bwe_dec.c \
		bwe/bwe_mdct.c \
		bwe/bwe_mdct_table.c \
        bwe/table.c

SRC_UTL       = util/bit_op.c \
		util/dsputil.c \
		util/mathtool.c \
		util/errexit.c \
		util/table_mathtool.c

SRC_AVQ       = avq/avq_cod.c \
		avq/avq_dec.c \
        avq/lead_deindexing.c \
        avq/lead_indexing.c \
		avq/re8_ppv.c \
		avq/re8_vor.c \
		avq/rom.c \
		avq/swb_avq_encode.c \
		avq/swb_avq_decode.c

SRCS      =	$(SRC_NSCORE) \
		$(SRC_G722CORE) \
		$(SRC_BWE) \
		$(SRC_UTL) \
		$(SRC_AVQ) \
		pcmswb/pcmswbenc.c \
		pcmswb/pcmswbdec.c \
		pcmswb/prehpf.c \
		pcmswb/qmfilt.c \
		pcmswb/softbit.c \
		pcmswb/table_qmfilt.c

OBJS =   $(SRCS:%.c=%.o)

all:		$(LIBRARY)

$(LIBRARY):	$(OBJS)
		@echo  $(LIBRARY)
		ar cru $(LIBRARY) $(OBJS)
		ranlib $(LIBRARY)

$(OBJS): $(HDRS)

clean:;		@rm -f $(LIBRARY) $(OBJS) core *.ln

.c.o:
	$(CC) $(CFLAGS) -c $< -o $@
