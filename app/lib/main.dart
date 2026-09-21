import 'package:flutter/material.dart';
import 'package:http/http.dart' as http;

import 'order_screen.dart';

class TwineApp extends StatelessWidget {
  const TwineApp({super.key, this.client});

  final http.Client? client;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Twine',
      home: OrderScreen(client: client),
    );
  }
}

void main() {
  runApp(const TwineApp());
}
