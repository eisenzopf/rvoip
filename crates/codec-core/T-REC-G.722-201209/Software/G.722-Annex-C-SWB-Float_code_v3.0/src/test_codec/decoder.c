/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.00 (2012-05)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <limits.h>
#include "pcmswb.h"
#include "softbit.h"

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
  fprintf(stderr, "\n");
  fprintf(stderr, " Options:\n");
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

  while (argc > 1 && argv[1][0] == '-') {
    /* check law character */
    if (strcmp(argv[1], "-quiet") == 0) {
      /* Set the quiet mode flag */
      params->quiet=1;

      /* Move arg{c,v} over the option to the next argument */
      argc--;
      argv++;
    }
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
    default : fprintf(stderr, "Error: Inconsitency in core and bitrate.\n");
    usage(progname); exit(-1);
  }

  return;
}

/***************************************************************************
* main()
***************************************************************************/
int
main(int argc, char *argv[])
{
  DECODER_PARAMS  params;
  int             nbitsIn;
  int nbitsIn_commandline; /*to memorise the command line bitrate */
  int             nbytesIn;
  int             nsamplesOut=0;
  FILE            *fpcode, *fpout;

  void            *theDecoder=0;
  int             status;
  short           sbufOut[NSamplesPerFrame32k];
  unsigned short  sbufIn[G192_HeaderSize+MaxBitsPerFrame];
  unsigned char   cbufIn[MaxBytesPerFrame];
  int             payloadsize;
  int             ploss_status=0;
  int i;

  for( i=0 ; i<NSamplesPerFrame32k ; i++ ){
	  sbufOut[i] = 0;
  }

  /* Set parameters from argv[]. */
  get_commandline_params( argc, argv, &params );

  switch (params.mode_bst) {
    case MODE_R00wm : nbitsIn = NBITS_MODE_R00wm; break;
    case MODE_R0wm  : nbitsIn = NBITS_MODE_R0wm;  break;
    case MODE_R1wm  : nbitsIn = NBITS_MODE_R1wm;  break;
    case MODE_R1sm  : nbitsIn = NBITS_MODE_R1sm;  break;
    case MODE_R2sm  : nbitsIn = NBITS_MODE_R2sm;  break;
    case MODE_R3sm  : nbitsIn = NBITS_MODE_R3sm;  break;
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
  theDecoder = pcmswbDecode_const(params.mode_dec);

  if (theDecoder == 0) {
    fprintf(stderr, "Decoder init error.\n");
    exit(1);
  }

  /* Reset (unnecessary if right after instantiation!). */
  pcmswbDecode_reset( theDecoder );

  while (1)
  {
    if( params.format == 0 )    /* G.192 softbit output format */
    {
      /* Read bitstream. */
      int nbitsIn_bst;
      nbitsIn = nbitsIn_commandline; /*reset in nbitsIn the command line bitrate*/
      nbytesIn = nbitsIn/CHAR_BIT;

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

	  pcmswbDecode_set(params.mode_dec, theDecoder);

      /* Check FER and payload size */
      payloadsize = checksoftbit( sbufIn );

      ploss_status = 0; /* False: No FER */
      if( payloadsize <= 0 )  /* Frame erasure */
      {
        ploss_status = 1; /* True: FER */
      }

      /* Convert from softbit to hardbit */
      softbit2hardbit( nbytesIn, &sbufIn[G192_HeaderSize], cbufIn );
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
    status = pcmswbDecode( cbufIn, sbufOut, theDecoder, ploss_status );

    if ( status ) {
      fprintf(stderr, "Decoder NG. Exiting.\n");
      exit(1);
    }

    /* Write output signal to fout. */
    fwrite(sbufOut, sizeof(short), nsamplesOut, fpout);
  }

  /* Close files. */
  fclose(fpcode);
  fclose(fpout);

  /* Delete the decoder. */
  pcmswbDecode_dest( theDecoder );

  return 0;
}
