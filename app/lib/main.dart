import 'package:flutter/material.dart';
import 'package:http/http.dart' as http;

import 'order_screen.dart';

class TwineApp extends StatelessWidget {
  const TwineApp({super.key, this.client, this.initialUrl});

  final http.Client? client;
  final String? initialUrl;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Twine',
      home: OrderScreen(client: client, initialUrl: initialUrl),
    );
  }
}

void main() {
  runApp(const TwineApp());
}
