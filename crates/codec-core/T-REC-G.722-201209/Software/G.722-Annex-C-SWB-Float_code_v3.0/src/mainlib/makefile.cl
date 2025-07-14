#
# Makefile for Microsoft Visual C++: nmake -f makefile.cl
#
.SUFFIXES: .obj .c .h

CC=cl

INCLUDES=  -Iutil -Ipcmswb -Ins -Ig722 -Ibwe -Iavq

DFLAGS=

CFLAGS= -nologo -EHsc -O2 -W3 $(INCLUDES) $(DFLAGS)

# Do not change the library name.
LIBRARY= pcmswb.lib

# List header files.
HDRS=

# List object files.
OBJS= ns/lpctool.obj \
      ns/autocorr_ns.obj \
      ns/table_lowband.obj \
      g722/funcg722.obj \
      g722/g722.obj \
      g722/g722_plc.obj \
      g722/g722_tables.obj \
      g722/g722_plc_tables.obj \
      g722/lsbcod_ns.obj \
      g722/hsb_enh.obj \
      g722/ns_common.obj \
      bwe/bwe_enc.obj \
      bwe/bwe_dec.obj \
      bwe/bwe_mdct.obj \
      bwe/bwe_mdct_table.obj \
      bwe/table.obj \
      util/bit_op.obj \
      util/errexit.obj \
      util/floatutil.obj \
      avq/avq_cod.obj \
      avq/avq_dec.obj \
      avq/lead_deindexing.obj \
      avq/lead_indexing.obj \
      avq/re8_ppv.obj \
      avq/re8_vor.obj \
      avq/rom.obj \
      avq/swb_avq_encode.obj \
      avq/swb_avq_decode.obj \
      pcmswb/pcmswbenc.obj \
      pcmswb/pcmswbdec.obj \
      pcmswb/prehpf.obj \
      pcmswb/qmfilt.obj \
      pcmswb/softbit.obj \
      pcmswb/table_qmfilt.obj \

all: $(LIBRARY)

$(LIBRARY): $(OBJS)
  del $(LIBRARY)
  lib -nologo -OUT:$(LIBRARY) $(OBJS)

$(OBJS): $(HDRS)

clean:
  del /S *.obj *.exe *.lib

.c.obj:
  $(CC) -c $(CFLAGS) $*.c -Fo$*.obj
.cpp.obj:
    $(CC) -c $(CFLAGS) $*.cpp -Fo$*.obj