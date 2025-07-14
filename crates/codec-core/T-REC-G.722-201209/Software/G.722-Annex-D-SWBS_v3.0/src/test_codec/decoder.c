/* ITU G.722 3rd Edition (2012-09) */

/*-----------------------------------------------------------------------------------
 ITU-T G.722-SWBS / G.722 Annex D - Reference C code for fixed-point implementation          
 Version 1.0
 Copyright (c) 2012,
 Huawei Technologies
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
  fprintf(stderr, " Usage: %s [-options] <codefile> <outfile> <bitrate(kbit/s/ch)> [-bitrateswitch <mode>]\n", progname);
  fprintf(stderr, "\n");
  fprintf(stderr, " where:\n" );
  fprintf(stderr, "   codefile     is the name of the output bitstream file.\n");
  fprintf(stderr, "   outfile      is the name of the decoded output file.\n");
  fprintf(stderr, "   bitrate      is the maximum decoded bitrate per channel:\n");
  fprintf(stderr, "                 \"64 (R1sm)\"              for G.722 core at 56 kbit/s,\n");
  fprintf(stderr, "                 \"96 (R3sm)\", \"80 (R2sm)\" for G.722 core at 64 kbit/s.\n");
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
  fprintf(stderr, "  -stereo  indicates that input signal is either mono,\n");
  fprintf(stderr, "                , or stereo (default is mono).\n");
#endif
  fprintf(stderr, "   -quiet       quiet processing.\n");
  fprintf(stderr, "   -bitrateswitch mode where mode is \n");
  fprintf(stderr, "                \"0\" to indicate that switching occurs between R3sm, R2sm, and G.722 at 64 kbit/s,\n");
  fprintf(stderr, "                \"1\" to indicate that switching occurs between R1sm and G.722 at 56 kbit/s,\n");
  fprintf(stderr, "\n");
}

typedef struct {
  int  quiet;
  int  mode_bst;
  int  mode_dec;
  char *code_fname;
  char *output_fname;
  int  format;
  int bitrateswitch;
#ifdef LAYER_STEREO
  short channel;
#endif
} DECODER_PARAMS;

static void  get_commandline_params(
                                    int            argc,
                                    char           *argv[],
                                    DECODER_PARAMS *params
                                    ) 
{
  char  *progname=argv[0];

  if (argc < 4) {
    fprintf(stderr, "Error: Too few arguments.\n");
    usage(progname);
    exit(1);
  }

  /* Default mode */
  params->quiet = 0;
  params->format = 0;    /* Default is G.192 softbit format */
  params->mode_dec = -1;
  params->mode_bst = -1;
  params->bitrateswitch = -1;
#ifdef LAYER_STEREO
  params->channel = 1;
#endif

  while (argc > 1 && argv[1][0] == '-') {
    /* check law character */
    if (strcmp(argv[1], "-quiet") == 0) {
      /* Set the quiet mode flag */
      params->quiet=1;

      /* Move arg{c,v} over the option to the next argument */
      argc--;
      argv++;
    }
#ifdef LAYER_STEREO
    else if (strcmp(argv[1], "-stereo") == 0) {
      /* Set the stereo mode flag */
      params->channel = 2;

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

  /* Open input code, output signal files. */
  params->code_fname   = argv[1];
  params->output_fname = argv[2];
#ifdef LAYER_STEREO
  if(params->channel == 1)
  {
#endif
  /* bitrate */
  if (strcmp(argv[3], "64") == 0) {
    params->mode_bst = MODE_R1sm;
  }
  else if (strcmp(argv[3], "80") == 0) {
    params->mode_bst = MODE_R2sm;
  }
  else if (strcmp(argv[3], "96") == 0) {
    params->mode_bst = MODE_R3sm;
  }
  else {
    fprintf(stderr, "Error: Invalid bitrate number %s\n", argv[3]);
    fprintf(stderr, "                          \"64\"         for G.722 core at 56 kbit/s,\n");
    fprintf(stderr, "                          \"96\" or \"80\" for G.722 core at 64 kbit/s.\n");
    usage(progname);
    exit(-1);
  }
#ifdef LAYER_STEREO
  }
  else
  {
      /* bitrate */
      if (strcmp(argv[3], "64") == 0) {
        params->mode_bst = MODE_R1ws;
      }
      else if (strcmp(argv[3], "80") == 0) {
        params->mode_bst = MODE_R2ss;/*shoude be decided by smaple rate further*/
      }
      else if (strcmp(argv[3], "96") == 0) {
        params->mode_bst = MODE_R3ss;
      }
      else if (strcmp(argv[3], "112") == 0) {
        params->mode_bst = MODE_R4ss;
      }
      else if (strcmp(argv[3], "128") == 0) {
        params->mode_bst = MODE_R5ss;
      }
      else {
        fprintf(stderr, "Error: Invalid bitrate number %s\n", argv[3]);
        fprintf(stderr, "                 \"64\" for G.722 wb stereo core at 56 kbit/s,\n");
        fprintf(stderr, "                 \"80\" for G.722 swb stereo core at 56 kbit/s.\n");
        fprintf(stderr, "                 \"96\" for G.722 swb stereo core at 64 kbit/s.\n");
        fprintf(stderr, "                 \"112\" for G.722 swb stereo core at 64 kbit/s.\n");
        fprintf(stderr, "                 \"128\" for G.722 swb stereo core at 64 kbit/s.\n");
        usage(progname);
        exit(-1);
      }
      
  }
#endif
  if(argc > 5) /*to have argv[4] and [5] */
  {
    if (strcmp(argv[4], "-bitrateswitch") == 0) {
      if (strcmp(argv[5], "0") == 0) {
        params->bitrateswitch = 0;
      }
      else if (strcmp(argv[5], "1") == 0) {
        params->bitrateswitch = 1;
      }
      else {
        fprintf(stderr, "Error: Invalid mode number %s\n", argv[4]);
        fprintf(stderr, "  Mode must be either \"0\" ,\n");
        fprintf(stderr, "               or     \"1\" \n");
        /* Display help message */
        usage(progname);
        exit(-1);
      }
    }
  }
  params->mode_dec = params->mode_bst;

  /* check for core/mode compatibility */
  switch (params->mode_dec) 
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
short Id_st_dec = -1;
short Id_st_dec_swb = -1;
short Id_st_pos = -1;
short Id_dmx = -1;
short Id_dmx_swb = -1;
short Id_fft = -1;
short Id_ifft = -1;
short Id_st_enc = -1;
short Id_st_enc_swb = -1;
short Id_itd = -1;
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
  DECODER_PARAMS  params;
  int             nbitsIn;
  int             nbitsIn_commandline; /*to memorise the command line bitrate */
  int             nbytesIn;
  int             nsamplesOut=0;
  FILE            *fpcode, *fpout;

  void            *theDecoder=0;
  int             status;
#ifdef LAYER_STEREO
  short           sbufOut[2*NSamplesPerFrame32k];
#else
  short           sbufOut[NSamplesPerFrame32k];
#endif
  unsigned short  sbufIn[G192_HeaderSize+MaxBitsPerFrame];
  unsigned char   cbufIn[MaxBytesPerFrame];
  int             payloadsize;
  int             ploss_status=0;
#ifdef LAYER_STEREO
  short           mode;
  short           highest_mode;
#endif

  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_INIT();
#endif 
  /*****************************/

  /* Set parameters from argv[]. */
  get_commandline_params( argc, argv, &params );
#ifdef LAYER_STEREO
  highest_mode = -1;
  if(params.bitrateswitch != -1)
      highest_mode = params.mode_bst;
#endif

  switch (params.mode_bst) {
  case MODE_R00wm : nbitsIn = NBITS_MODE_R00wm; break;
  case MODE_R0wm  : nbitsIn = NBITS_MODE_R0wm;  break;
  case MODE_R1wm  : nbitsIn = NBITS_MODE_R1wm;  break;
  case MODE_R1sm  : nbitsIn = NBITS_MODE_R1sm;  break;
  case MODE_R2sm  : nbitsIn = NBITS_MODE_R2sm;  break;
  case MODE_R3sm  : nbitsIn = NBITS_MODE_R3sm;  break;
#ifdef LAYER_STEREO
  case MODE_R1ws  : nbitsIn = NBITS_MODE_R1ws;  break;
  case MODE_R2ws  : nbitsIn = NBITS_MODE_R2ws;  break;
  case MODE_R2ss  : nbitsIn = NBITS_MODE_R2ss;  break;
  case MODE_R3ss  : nbitsIn = NBITS_MODE_R3ss;  break;
  case MODE_R4ss  : nbitsIn = NBITS_MODE_R4ss;  break;
  case MODE_R5ss  : nbitsIn = NBITS_MODE_R5ss;  break;
#endif
  default : fprintf(stderr, "Mode specification error.\n"); exit(-1);
  }
  nbitsIn_commandline = nbitsIn; /*memorise the command line bitrate not yet modified*/
  nbytesIn = nbitsIn/CHAR_BIT;

  switch (params.mode_dec) 
  {
    case MODE_R00wm : nsamplesOut = NSamplesPerFrame16k; break;
    case MODE_R0wm  : nsamplesOut = NSamplesPerFrame16k; break;
    case MODE_R1wm  : nsamplesOut = NSamplesPerFrame16k; break;
    case MODE_R1sm  : nsamplesOut = NSamplesPerFrame32k; break;
    case MODE_R2sm  : nsamplesOut = NSamplesPerFrame32k; break;
    case MODE_R3sm  : nsamplesOut = NSamplesPerFrame32k; break;
#ifdef LAYER_STEREO
    case MODE_R1ws  : nsamplesOut = NSamplesPerFrame16k * 2;  break;
    case MODE_R2ws  : nsamplesOut = NSamplesPerFrame16k * 2;  break;
    case MODE_R2ss  : nsamplesOut = NSamplesPerFrame32k * 2;  break;
    case MODE_R3ss  : nsamplesOut = NSamplesPerFrame32k * 2;  break;
    case MODE_R4ss  : nsamplesOut = NSamplesPerFrame32k * 2;  break;
    case MODE_R5ss  : nsamplesOut = NSamplesPerFrame32k * 2;  break;
#endif
  default : fprintf(stderr, "Mode specification error.\n"); exit(-1);
  }

  /* Open input bitstream */
  fpcode   = fopen(params.code_fname, "rb");
  if (fpcode == (FILE *)NULL) {
    fprintf(stderr, "file open error.\n");
    exit(1);
  }

  /* Open output speech file. */
  fpout  = fopen(params.output_fname, "wb");
  if (fpout == (FILE *)NULL) {
    fprintf(stderr, "file open error.\n");
    exit(1);
  }

  /* Instanciate an decoder. */
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_PUSH(0, "dummy"); /* count static memories */
#endif
  /*****************************/  
  theDecoder = pcmswbDecode_const((Word16)params.mode_dec);
  if (theDecoder == 0) {
    fprintf(stderr, "Decoder init error.\n");
    exit(1);
  }
#ifdef WMOPS_ALL
  setFrameRate(32000, NSamplesPerFrame32k);
  Id = (short)getCounterId("Decoder");
  setCounter(Id);
  Init_WMOPS_counter();
#endif
#ifdef WMOPS_IDX
  setFrameRate(32000, NSamplesPerFrame32k);
  Id = (short)getCounterId("rest code");
  setCounter(Id);
  Init_WMOPS_counter();
  Id_st_dec = (short)getCounterId("g722_stereo_decode_fx");
  setCounter(Id_st_dec);
  Init_WMOPS_counter();
  Id_st_dec_swb = (short)getCounterId("g722_stereo_decoder_swb_fx");
  setCounter(Id_st_dec_swb);
  Init_WMOPS_counter();
  Id_st_pos = (short)getCounterId("stereo_dec_timepos_fx");
  setCounter(Id_st_pos);
  Init_WMOPS_counter();
  Id_fft = (short)getCounterId("FFT");
  setCounter(Id_fft);
  Init_WMOPS_counter();
  Id_ifft = (short)getCounterId("IFFT");
  setCounter(Id_ifft);
  Init_WMOPS_counter();
  Id_itd = (short)getCounterId("stereo_synthesis_fx");
  setCounter(Id_itd);
  Init_WMOPS_counter();
#endif

  /* Reset (unnecessary if right after instantiation!). */
  pcmswbDecode_reset( theDecoder );

  while (1)
  {
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
   setCounter(Id_st_dec);
    fwc();
    Reset_WMOPS_counter();
   setCounter(Id_st_dec_swb);
    fwc();
    Reset_WMOPS_counter();
   setCounter(Id_st_pos);
    fwc();
    Reset_WMOPS_counter();
   setCounter(Id_fft);
    fwc();
    Reset_WMOPS_counter();
   setCounter(Id_ifft);
    fwc();
    Reset_WMOPS_counter();
   setCounter(Id_itd);
    fwc();
    Reset_WMOPS_counter();
   setCounter(Id);
#endif
    if( params.format == 0 )    /* G.192 softbit output format */
    {
      /* Read bitstream. */
      int nbitsIn_bst;
      nbitsIn = nbitsIn_commandline; /*reset in nbitsIn the command line bitrate*/
      nbytesIn = nbitsIn/CHAR_BIT;
#ifdef LAYER_STEREO
      if(params.channel == 1)
      {
#endif
      if (fread(sbufIn, sizeof(short), G192_HeaderSize, fpcode) <  G192_HeaderSize)
        break;
      nbitsIn_bst = sbufIn[1];

      nsamplesOut = NSamplesPerFrame32k; /*only SWB (32 kHz sampled) output, even for WB orNB, for witching*/
      if (fread(sbufIn+G192_HeaderSize, sizeof(short), nbitsIn_bst, fpcode) !=  (unsigned) nbitsIn_bst)
        break;
      if(nbitsIn_bst < nbitsIn)
      {
        nbitsIn = nbitsIn_bst; /* min of the 2*/
        nbytesIn = nbitsIn/CHAR_BIT;
      }

      if (params.bitrateswitch == -1) /*default mode, no bitrateswitch in command line*/
      {                               /*valide rates R1sm, R2sm, R3sm & R4sm, only SWB*/
        switch (nbitsIn) 
        {
        case 320  : 
          params.mode_dec = params.mode_bst = MODE_R1sm;
          break; /*MODE_R1sm*/
        case 400  : 
          params.mode_dec = params.mode_bst = MODE_R2sm;
          break; /*MODE_R2sm*/
        case 480  : 
          params.mode_dec = params.mode_bst = MODE_R3sm;
          break; /*MODE_R3sm*/
        default : 
          fprintf(stderr, "Error: bitrate (%d kbps) not supported for G722 core.\n", nbitsIn_bst/5);
          exit(-1);
        }
      }
      else if (params.bitrateswitch == 0) /*switching between R3sm, R2sm, and G.722 at 64 kbit/s*/
      {                                   /* only G.722 core, output always SWB (32kHz)*/
        switch (nbitsIn) 
        {
        case 320  : 
          params.mode_dec = params.mode_bst = MODE_R1wm;
          break; /*MODE_R1wm*/
        case 400  : 
          params.mode_dec = params.mode_bst = MODE_R2sm;
          break; /*MODE_R2sm*/
        case 480  : 
          params.mode_dec = params.mode_bst = MODE_R3sm;
          break; /*MODE_R3sm*/
        default : 
          fprintf(stderr, "Error: bitrate (%d kbps) not supported for G722 core.\n", nbitsIn_bst/5);
          exit(-1);
        }
      }
      else                               /*switching between R1sm and G.722 at 56 kbit/s*/
      {                                  /* only G.722 core, output always SWB (32kHz)*/
        switch (nbitsIn) 
        {
        case 280  : 
          params.mode_dec = params.mode_bst = MODE_R0wm;
          break; /*MODE_R0wm*/
        case 320  : 
          params.mode_dec = params.mode_bst = MODE_R1sm;
          break; /*MODE_R1sm*/
        default : 
          fprintf(stderr, "Error: bitrate (%d kbps) not supported for G722 core.\n", nbitsIn_bst/5);
          exit(-1);
        }
      }
      pcmswbDecode_set((Word16)params.mode_dec, theDecoder);
#ifdef LAYER_STEREO
      }
      else
      {
        if (fread(sbufIn, sizeof(short), G192_HeaderSize, fpcode) <  G192_HeaderSize)
          break;

        nbitsIn_bst = sbufIn[1];

        if (fread(sbufIn+G192_HeaderSize, sizeof(short), nbitsIn_bst, fpcode) !=  (unsigned) nbitsIn_bst)
          break;

        if(nbitsIn_bst < nbitsIn)
        {
          nbitsIn = nbitsIn_bst; /* min of the 2*/
          nbytesIn = nbitsIn/CHAR_BIT;
        }

        if (params.bitrateswitch == -1) /*default mode, no bitrateswitch in command line*/
        {                               /*valide rates R1ws,R2ss, R3ss, R4ss & R5ss */
          switch (nbitsIn) {
            case 320: 
              params.mode_dec = params.mode_bst = MODE_R1ws;
              break; /*MODE_R1ws*/
            case 400: 
              params.mode_dec = params.mode_bst = MODE_R2ss;
              break; /*MODE_R2ss*/
            case 480: 
              params.mode_dec = params.mode_bst = MODE_R3ss;
              break; /*MODE_R3ss*/
            case 560: 
              params.mode_dec = params.mode_bst = MODE_R4ss;
              break; /*MODE_R4ss*/
            case 640: 
              params.mode_dec = params.mode_bst = MODE_R5ss;
              break; /*MODE_R5ss*/
            default: 
              fprintf(stderr, "Error: bitrate (%d kbps) not supported for G722 core.\n", nbitsIn_bst/5);
              exit(-1);
          }
        }
        else if (params.bitrateswitch == 0) /*switching between R5ss, R4ss G.722 at 64 kbit/s*/
        {    
          switch (nbitsIn) {
            case 320:
              {
                switch (highest_mode) {
                  case MODE_R5ss:
                    params.mode_dec = params.mode_bst = MODE_R1wm;
                    break;
                  case MODE_R4ss:
                    params.mode_dec = params.mode_bst = MODE_R1wm;
                    break;
                  case MODE_R3ss:
                    params.mode_dec = params.mode_bst = MODE_R1wm;
                    break;
                  case MODE_R2ws:
                    params.mode_dec = params.mode_bst = MODE_R1wm;
                    break;
                  case MODE_R2ss:
                    highest_mode = MODE_R2ws;
                    params.mode_dec = params.mode_bst = MODE_R1wm;
                    break;
                  case MODE_R1ws:
                    params.mode_dec = params.mode_bst = MODE_R1ws;
                    break;
                }
              }
              break;
            case 400:
              {
                switch (highest_mode) {
                  case MODE_R5ss:
                    params.mode_dec = params.mode_bst = MODE_R2sm;
                    break;
                  case MODE_R4ss:
                    params.mode_dec = params.mode_bst = MODE_R2sm;
                    break;
                  case MODE_R3ss:
                    params.mode_dec = params.mode_bst = MODE_R2sm;
                    break;
                  case MODE_R2ws:
                    params.mode_dec = params.mode_bst = MODE_R2ws;
                    break;
                  case MODE_R2ss:
                    params.mode_dec = params.mode_bst = MODE_R2ss;
                    break;
                }
              }
              break;
            case 480:
              {
                switch (highest_mode) {
                  case MODE_R5ss:
                    params.mode_dec = params.mode_bst = MODE_R3sm;
                    break;
                  case MODE_R4ss:
                    params.mode_dec = params.mode_bst = MODE_R3sm;
                    break;
                  case MODE_R3ss:
                    params.mode_dec = params.mode_bst = MODE_R3ss;
                    break;
                }
              }
              break;
            case 560: 
              params.mode_dec = params.mode_bst = MODE_R4ss;
              break; /*MODE_R4ss*/
            case 640: 
              params.mode_dec = params.mode_bst = MODE_R5ss;
              break; /*MODE_R5ss*/
            default: 
              fprintf(stderr, "Error: bitrate (%d kbps) not supported for G722 core.\n", nbitsIn_bst/5);
              exit(-1);
          }
        }
        else if (params.bitrateswitch == 1) /*switching between R5ss, R4ss G.722 at 56 kbit/s*/
        { 
          switch (nbitsIn) {
            case 280:
              {
                switch (highest_mode) {
                  case MODE_R2ss:
                    params.mode_dec = params.mode_bst = MODE_R0wm;
                    break;
                  case MODE_R1sm:
                    params.mode_dec = params.mode_bst = MODE_R0wm;
                    break;
                  case MODE_R1ws:
                    params.mode_dec = params.mode_bst = MODE_R0wm;
                    break;
                }
              }
              break;
            case 320:
              {
                switch (highest_mode) {
                  case MODE_R2ss:
                    params.mode_dec = params.mode_bst = MODE_R1sm;
                    break;
                  case MODE_R1sm:
                    params.mode_dec = params.mode_bst = MODE_R1sm;
                    break;
                  case MODE_R1ws:
                    params.mode_dec = params.mode_bst = MODE_R1ws;
                    break;
                }
              }
              break;
            case 400:
              {
                switch (highest_mode) {
                  case MODE_R2ss:
                    params.mode_dec = params.mode_bst = MODE_R2ss;
                    break;
                }
              }
              break;
            default: 
              fprintf(stderr, "Error: bitrate (%d kbps) not supported for G722 core.\n", nbitsIn_bst/5);
              exit(-1);
          }
        }
        pcmswbDecode_set((Word16)params.mode_dec, theDecoder);
      }
#endif

      /* Check FER and payload size */
      payloadsize = checksoftbit( sbufIn );

      ploss_status = 0; /* False: No FER */
      IF( payloadsize <= 0 )  /* Frame erasure */
      {
        ploss_status = 1; /* True: FER */
      }

      /* Convert from softbit to hardbit */
      softbit2hardbit( (Word16)nbytesIn, &sbufIn[G192_HeaderSize], cbufIn );
    }
    else
    {
      /* Read bitstream. */
      if (fread(cbufIn, sizeof(char), nbytesIn, fpcode) ==  0)
        break;
      ploss_status = 0; /* False: No FER */
      /* When FER is detected, set ploss_status=1 */
    }

    /* Decode. */
        status = pcmswbDecode( cbufIn, sbufOut, theDecoder, (Word16)ploss_status 
#ifdef LAYER_STEREO
                              , &mode, &highest_mode
#endif
            );
    if ( status ) {
      fprintf(stderr, "Decoder NG. Exiting.\n");
      exit(1);
    }
#ifdef LAYER_STEREO
    if(mode == MODE_R2ws)
    {
      nsamplesOut = NSamplesPerFrame16k * 2;
    }
#endif
    /* Write output signal to fout. */
    fwrite(sbufOut, sizeof(short), nsamplesOut, fpout);
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
  setCounter(Id_st_dec);
  fwc();
  WMOPS_output(0);
  setCounter(Id_st_dec_swb);
  fwc();
  WMOPS_output(0);
  setCounter(Id_st_pos);
  fwc();
  WMOPS_output(0);
  setCounter(Id_fft);
  fwc();
  WMOPS_output(0);
  setCounter(Id_ifft);
  fwc();
  WMOPS_output(0);
  setCounter(Id_itd);
  fwc();
  WMOPS_output(0);
#ifndef SUPPRESS_COUNTER_RESULTS
  //WMOPS_output(0);
#endif
#endif

  /* Close files. */
  fclose(fpcode);
  fclose(fpout);

  /* Delete the decoder. */
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_POP();
#endif
  /*****************************/ 
  pcmswbDecode_dest( theDecoder );
  /*****************************/
#ifdef DYN_RAM_CNT
  DYN_RAM_REPORT();
#endif 
  /*****************************/
  return 0;
}
