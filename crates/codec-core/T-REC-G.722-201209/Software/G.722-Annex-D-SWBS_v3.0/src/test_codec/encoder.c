/* ITU G.722 3rd Edition (2012-09) */

/*-----------------------------------------------------------------------------------
 ITU-T G.722-SWBS / G.722 Annex D - Reference C code for fixed-point implementation          
 Version 1.0
 Copyright (c) 2012,
 Huawei Technologies, France Telecom
-----------------------------------------------------------------------------------*/

/*--------------------------------------------------------------------------
 ITU-T Annex B (ex G.722-SWB) Source Code
 Software Release 1.00 (2010-09)
 (C) 2010 France Telecom, Huawei Technologies, NTT, VoiceAge Corp.
--------------------------------------------------------------------------*/

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <limits.h>
#include "stl.h"
#include "pcmswb.h"
#include "softbit.h"
#ifdef LAYER_STEREO
#include "g722_stereo.h"
#endif
/*****************************/
#ifdef DYN_RAM_CNT
#define MAIN_ROUTINE
#include "dyn_ram_cnt.h"
#endif
/*****************************/


/***************************************************************************
* usage()
***************************************************************************/
static void usage(char progname[])
{
  fprintf(stderr, "\n");
  fprintf(stderr, " Usage: %s [-options] <infile> <codefile> <bitrate>\n", progname);
  fprintf(stderr, "\n");
  fprintf(stderr, " where:\n" );
  fprintf(stderr, "   infile       is the name of the input file to be encoded.\n");
  fprintf(stderr, "   codefile     is the name of the output bitstream file.\n");
  fprintf(stderr, "   bitrate      is the desired bitrate:\n");
  fprintf(stderr, "                 \"64\" (R1sm)              for G.722 core at 56 kbit/s,\n");
  fprintf(stderr, "                 \"80\" (R2sm), \"96\" (R3sm) for G.722 core at 64 kbit/s.\n");
#ifdef LAYER_STEREO
  fprintf(stderr, "                 \"64\" for G.722 wb stereo core at 56 kbit/s,\n");
  fprintf(stderr, "                 \"80\" for G.722 wb stereo core at 64 kbit/s,\n");
  fprintf(stderr, "                 \"80\" for G.722 swb stereo core at 56 kbit/s.\n");
  fprintf(stderr, "                 \"96\" for G.722 swb stereo core at 64 kbit/s.\n");
  fprintf(stderr, "                 \"112\" for G.722 swb stereo core at 64 kbit/s.\n");
  fprintf(stderr, "                 \"128\" for G.722 swb stereo core at 64 kbit/s.\n");
#endif
  fprintf(stderr, "\n");
  fprintf(stderr, " Options:\n");
#ifdef LAYER_STEREO
  fprintf(stderr, "  -wb indicates that input signal is either narrow band,\n");
  fprintf(stderr, "                wideband, or superwideband (default is -swb).\n");
  fprintf(stderr, "  -stereo  indicates that input signal is either mono,\n");
  fprintf(stderr, "                , or stereo (default is mono).\n");
#endif
  fprintf(stderr, "   -quiet       quiet processing.\n");
  fprintf(stderr, "\n");
}

typedef struct {
  int  mode;
  int  quiet;
  int  format;
  unsigned short  inputSF;
  char *input_fname;
  char *code_fname;
#ifdef LAYER_STEREO
  short channel;
#endif
} ENCODER_PARAMS;

static void  get_commandline_params(
                                    int            argc,
                                    char           *argv[],
                                    ENCODER_PARAMS *params
                                    ) 
{
  char  *progname=argv[0];

  if (argc < 4) {
    fprintf(stderr, "Error: Too few arguments.\n");
    usage(progname);
    exit(1);
  }

  /* Default mode */
  params->mode = -1;
  params->quiet = 0;
  params->format = 0;        /* Default is G.192 softbit format */
  params->inputSF = 32000;   /* Default is super-wideband input */
#ifdef LAYER_STEREO
  params->channel = 1;
#endif

  /* Search options */
  while (argc > 1 && argv[1][0] == '-') {
    if (strcmp(argv[1],"-quiet") == 0) {
      /* Set the quiet mode flag */
      params->quiet=1;
      /* Move arg{c,v} over the option to the next argument */
      argc--;
      argv++;
    }
#ifdef LAYER_STEREO
    else if (strcmp(argv[1],"-stereo") == 0) {
      /* Set the quiet mode flag */
      params->channel=2;
      /* Move arg{c,v} over the option to the next argument */
      argc--;
      argv++;
    }
    else if (strcmp(argv[1],"-wb") == 0) {
      /* Set the quiet mode flag */
      params->inputSF=16000;
      /* Move arg{c,v} over the option to the next argument */
      argc--;
      argv++;
    }
#endif
    else if (strcmp(argv[1], "-h") == 0 || strcmp(argv[1], "-?") == 0) {
      /* Display help message */
      usage(progname);
      exit(1);
    }
    else {
      fprintf(stderr, "Error: Invalid option \"%s\"\n\n",argv[1]);
      usage(progname);
      exit(1);
    }
  }

  /* Open input signal and output code files. */
  params->input_fname  = argv[1];
  params->code_fname   = argv[2];
#ifdef LAYER_STEREO
  if(params->channel == 1)
  {
#endif
  /* bitrate */
  if (strcmp(argv[3], "64") == 0) {
    params->mode = MODE_R1sm;
  }
  else if (strcmp(argv[3], "80") == 0) {
    params->mode = MODE_R2sm;
  }
  else if (strcmp(argv[3], "96") == 0) {
    params->mode = MODE_R3sm;
  }
  else {
    fprintf(stderr, "Error: Invalid bitrate number %s\n", argv[3]);
    fprintf(stderr, "                           \"64\"         for G.722 core at 56 kbit/s,\n");
    fprintf(stderr, "                           \"96\" or \"80\" for G.722 core at 64 kbit/s.\n");
    usage(progname);
    exit(-1);
  }
#ifdef LAYER_STEREO
  }
  else
  {
      if(params->inputSF == 16000)
      {
          /* bitrate */
          if (strcmp(argv[3], "64") == 0) {
            params->mode = MODE_R1ws;
          }
          else if (strcmp(argv[3], "80") == 0) {
            params->mode = MODE_R2ws;
          }
          else {
            fprintf(stderr, "Error: Invalid bitrate number %s\n", argv[3]);
            fprintf(stderr, "                 \"64\" for G.722 wb stereo core at 56 kbit/s,\n");
            fprintf(stderr, "                 \"80\" for G.722 wb stereo core at 64 kbit/s,\n");
            usage(progname);
            exit(-1);
          }
      }
      else
      {
          /* bitrate */
          if (strcmp(argv[3], "80") == 0) {
            params->mode = MODE_R2ss;
          }
          else if (strcmp(argv[3], "96") == 0) {
            params->mode = MODE_R3ss;
          }
          else if (strcmp(argv[3], "112") == 0) {
            params->mode = MODE_R4ss;
          }
          else if (strcmp(argv[3], "128") == 0) {
            params->mode = MODE_R5ss;
          }
          else {
            fprintf(stderr, "Error: Invalid bitrate number %s\n", argv[3]);
            fprintf(stderr, "                 \"80\"  for G.722 swb stereo core at 56 kbit/s.\n");
            fprintf(stderr, "                 \"96\"  for G.722 swb stereo core at 64 kbit/s.\n");
            fprintf(stderr, "                 \"112\" for G.722 swb stereo core at 64 kbit/s.\n");
            fprintf(stderr, "                 \"128\" for G.722 swb stereo core at 64 kbit/s.\n");
            usage(progname);
            exit(-1);
          }
      }
  }
#endif


  /* check for core/mode compatibility */
  switch (params->mode) 
  {
  case MODE_R00wm : break;
  case MODE_R0wm  : break;
  case MODE_R1wm  : break;
  case MODE_R1sm  : break;
  case MODE_R2sm  : break;
  case MODE_R3sm  : break;
#ifdef LAYER_STEREO
  case MODE_R1ws  : break;
  case MODE_R2ws  : break;
  case MODE_R2ss  : break;
  case MODE_R3ss  : break;
  case MODE_R4ss  : break;
  case MODE_R5ss  : break;
#endif
  default : fprintf(stderr, "Error: Inconsitency in core and bitrate.\n");
    usage(progname); exit(-1);
  }


  return;
}

#ifdef WMOPS
short Id = -1;
short Id_dmx = -1;
short Id_dmx_swb = -1;
short Id_fft = -1;
short Id_ifft = -1;
short Id_st_enc = -1;
short Id_st_enc_swb = -1;
short Id_itd = -1;
short Id_st_dec = -1;
#endif

/*****************************/
#ifdef DYN_RAM_CNT
int           dyn_ram_level_cnt;
unsigned long *dyn_ram_table_ptr;
unsigned long dyn_ram_table[DYN_RAM_MAX_LEVEL];
char          dyn_ram_name_table[DYN_RAM_MAX_LEVEL][DYN_RAM_MAX_NAME_LENGTH];
unsigned long dyn_ram_current_value;
unsigned long dyn_ram_max_value;
unsigned long dyn_ram_max_counter;
#endif 
/*****************************/

/***************************************************************************
* main()
***************************************************************************/

int
main(int argc, char *argv[])
{
  int             i;
  ENCODER_PARAMS  params;
  int             nsamplesIn;
  int             nbitsOut;
  int             nbytesOut;
  FILE            *fpin, *fpcode;

  void            *theEncoder=0;

  int             status;
#ifdef LAYER_STEREO
  short           sbufIn[NSamplesPerFrame32k * 2];
#else
  short           sbufIn[NSamplesPerFrame32k];
#endif
  unsigned short  sbufOut[G192_HeaderSize+MaxBitsPerFrame];
  unsigned char   cbufOut[MaxBytesPerFrame];

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_INIT();
#endif
  /*****************************/

  /* Set parameters from argv[]. */
  get_commandline_params( argc, argv, &params );
#ifdef LAYER_STEREO
  if(params.channel == 1)
  {
#endif
  if ( params.inputSF == 8000 )
    nsamplesIn = NSamplesPerFrame08k; /* Input sampling rate is 8 kHz. */
  else if ( params.inputSF == 16000 )
    nsamplesIn = NSamplesPerFrame16k; /* Input sampling rate is 16 kHz. */
  else 
    nsamplesIn = NSamplesPerFrame32k; /* Input sampling rate is 32 kHz in default. */
#ifdef LAYER_STEREO
  }
  else
  {
      if ( params.inputSF == 16000 )
        nsamplesIn = NSamplesPerFrame16k * 2; /* Input sampling rate is 16 kHz. */
      else 
        nsamplesIn = NSamplesPerFrame32k * 2; /* Input sampling rate is 32 kHz in default. */
  }
#endif

  switch (params.mode) {
  case MODE_R00wm : nbitsOut = NBITS_MODE_R00wm; break;
  case MODE_R0wm  : nbitsOut = NBITS_MODE_R0wm;  break;
  case MODE_R1wm  : nbitsOut = NBITS_MODE_R1wm;  break;
  case MODE_R1sm  : nbitsOut = NBITS_MODE_R1sm;  break;
  case MODE_R2sm  : nbitsOut = NBITS_MODE_R2sm;  break;
  case MODE_R3sm  : nbitsOut = NBITS_MODE_R3sm;  break;
#ifdef LAYER_STEREO
  case MODE_R1ws  : nbitsOut = NBITS_MODE_R1ws;  break;
  case MODE_R2ws  : nbitsOut = NBITS_MODE_R2ws;  break;
  case MODE_R2ss  : nbitsOut = NBITS_MODE_R2ss;  break;
  case MODE_R3ss  : nbitsOut = NBITS_MODE_R3ss;  break;
  case MODE_R4ss  : nbitsOut = NBITS_MODE_R4ss;  break;
  case MODE_R5ss  : nbitsOut = NBITS_MODE_R5ss;  break;
#endif
  default : fprintf(stderr, "Mode specification error.\n"); exit(-1);
  }
  nbytesOut = nbitsOut/CHAR_BIT;

  /* Open input speech file. */
  fpin = fopen(params.input_fname, "rb");
  if (fpin == (FILE *)NULL) {
    fprintf(stderr, "file open error.\n");
    exit(1);
  }

  /* Open output bitstream. */
  fpcode = fopen(params.code_fname, "wb");
  if (fpcode == (FILE *)NULL) {
    fprintf(stderr, "file open error.\n");
    exit(1);
  }

  /* Instanciate an encoder. */
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH(0, "dummy"); /* count static memories */
#endif
  /*****************************/  
  theEncoder = pcmswbEncode_const(params.inputSF, (Word16)params.mode
#ifdef LAYER_STEREO
      ,params.channel
#endif
      );
  if (theEncoder == 0) {
    fprintf(stderr, "Encoder init error.\n");
    exit(1);
  }

  /* Reset (unnecessary if right after instantiation!). */
  pcmswbEncode_reset( theEncoder );
#ifdef WMOPS_ALL
   setFrameRate(32000, NSamplesPerFrame32k);
   Id = (short)getCounterId("Encoder");
   setCounter(Id);
   Init_WMOPS_counter();
#endif
#ifdef WMOPS_IDX
   setFrameRate(32000, NSamplesPerFrame32k);
   Id = (short)getCounterId("rest code");
   setCounter(Id);
   Init_WMOPS_counter();
   Id_dmx = getCounterId("downmix");
   setCounter(Id_dmx);
   Init_WMOPS_counter();
   Id_itd = getCounterId("get_interchannel_difference");
   setCounter(Id_itd);
   Init_WMOPS_counter();
   Id_st_enc = getCounterId("g722_stereo_encode");
   setCounter(Id_st_enc);
   Init_WMOPS_counter();
   Id_fft = getCounterId("FFT");
   setCounter(Id_fft);
   Init_WMOPS_counter();
   Id_ifft = getCounterId("iFFT");
   setCounter(Id_ifft);
   Init_WMOPS_counter();
   Id_dmx_swb = getCounterId("downmix_swb");
   setCounter(Id_dmx_swb);
   Init_WMOPS_counter();
   Id_st_enc_swb = getCounterId("G722_stereo_encoder_swb");
   setCounter(Id_st_enc_swb);
   Init_WMOPS_counter();
#endif

  while (1) {

#ifdef WMOPS_ALL
    setCounter(Id);
    fwc();
    Reset_WMOPS_counter();
    setCounter(Id);
#endif
#ifdef WMOPS_IDX
    setCounter(Id);
    fwc();
    Reset_WMOPS_counter();
    setCounter(Id_dmx);
    fwc();
    Reset_WMOPS_counter();
    setCounter(Id_itd);
    fwc();
    Reset_WMOPS_counter();
    setCounter(Id_st_enc);
    fwc();
    Reset_WMOPS_counter();
    setCounter(Id_st_enc_swb);
    fwc();
    Reset_WMOPS_counter();
    setCounter(Id_fft);
    fwc();
    Reset_WMOPS_counter();
    setCounter(Id_ifft);
    fwc();
    Reset_WMOPS_counter();
    setCounter(Id_dmx_swb);
    fwc();
    Reset_WMOPS_counter();
    setCounter(Id);
#endif
    /* Initialize sbuf[]. */
    for (i=0; i<nsamplesIn; i++) sbufIn[i] = 0;

    /* Read input singal from fin. */
    if ( fread( sbufIn, sizeof(short), nsamplesIn, fpin ) == 0 )
      break;

    /* Encode. */
    status = pcmswbEncode( sbufIn, cbufOut, theEncoder );

    if ( status ) {
      fprintf(stderr, "Encoder NG. Exiting.\n");
      exit(1);
    }


      if( params.format == 0 ) {   /* G.192 softbit output format */
        /* Write main header */
        sbufOut[0] = G192_SYNCHEADER;
        sbufOut[idxG192_BitstreamLength] = (unsigned short)nbitsOut;

        /* Convert from hardbit to softbit. */
        hardbit2softbit( (Word16)nbytesOut, cbufOut, &sbufOut[G192_HeaderSize] );

        /* Write bitstream. */
        fwrite( sbufOut, sizeof(short), G192_HeaderSize+nbitsOut, fpcode );
      }
      else {   /* Hardbit output format */
        /* Write bitstream. */
        fwrite( cbufOut, sizeof(char), nbytesOut, fpcode );
      }
    }
#ifdef WMOPS_ALL
   setCounter(Id);
   fwc();
   WMOPS_output(0);
#endif
#ifdef WMOPS_IDX
   setCounter(Id);
   fwc();
   WMOPS_output(0);
   setCounter(Id_dmx);
   fwc(); 
   WMOPS_output(0);
   setCounter(Id_itd);
   fwc(); 
   WMOPS_output(0);
   setCounter(Id_fft);
   fwc(); 
   WMOPS_output(0);
   setCounter(Id_ifft);
   fwc(); 
   WMOPS_output(0);
   setCounter(Id_st_enc);
   fwc(); 
   WMOPS_output(0);
   setCounter(Id_st_enc_swb);
   fwc(); 
   WMOPS_output(0);
   setCounter(Id_dmx_swb);
   fwc(); 
   WMOPS_output(0);
#ifndef SUPPRESS_COUNTER_RESULTS
 // WMOPS_output(0);
#endif
#endif

  /* Close files. */
  fclose(fpin);
  fclose(fpcode);

  /* Delete the encoder. */
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 
  pcmswbEncode_dest( theEncoder );

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_REPORT();
#endif 
  /*****************************/

  return 0;
}
